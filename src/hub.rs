use log::{error, info, trace};
use mio::event::Event;
use mio::{Events, Interest, Poll, Token};
use signal_hook::consts::signal::{SIGINT, SIGTERM, SIGUSR1};
use signal_hook_mio::v1_0::Signals;
use std::collections::HashMap;
use std::io::{Result, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::io::TcpServer;
use crate::keybind::Action;
use crate::monitor::DeviceMonitor;
use crate::traits::{
    IoInstance, IoResult, TOKEN_DEV, TOKEN_DYNAMIC_START, TOKEN_MONITOR_SERVER, TOKEN_RO_SERVER,
    TOKEN_SERVER, TOKEN_SIGNAL,
};

/// Configuration for the read-write server that supports force-release.
pub struct RwConfig {
    /// The configured bind port. 0 means "OS-assigned random port"; on each
    /// force-release a new random port is obtained. A fixed port is re-bound
    /// as-is (still disconnecting all RW clients).
    pub bind_port: u16,
    /// Optional file to publish the current RW port (and pid) to. Written on
    /// startup and after every force-release; deleted on graceful shutdown.
    pub port_file: Option<PathBuf>,
}

pub struct IoHub {
    poll: Poll,
    instances: HashMap<Token, Box<dyn IoInstance>>,

    // The device is special, which is why we do not want it as part of the
    // instances (despite it is has a compatible type).
    device: Box<dyn IoInstance>,

    /// Read-write server: bidirectional clients (bots/users). Rebuilt on
    /// force-release. `None` if no RW server was configured.
    server: Option<TcpServer>,

    /// Configuration used to re-bind the RW server on force-release and to
    /// publish its port. `None` when there is no RW server.
    rw_config: Option<RwConfig>,

    /// Read-only server: clients receive a raw mirror of the device output but
    /// their input is discarded. Never affected by force-release.
    ro_server: Option<TcpServer>,

    monitor: Option<DeviceMonitor>,

    signals: Signals,

    quit_requested: bool,

    announce: bool,

    /// When true the device's send buffer is full.  We stop reading from
    /// clients so that TCP backpressure propagates all the way to the
    /// senders.  Cleared when the device fires a WRITABLE event.
    device_write_blocked: bool,

    /// Bytes that could not be written to the device during a partial write.
    /// Flushed first when the device becomes writable again.
    pending_device_write: Vec<u8>,

    /// Last status message for the device (e.g. Connected or Error)
    last_device_status_msg: Option<String>,

    /// Template for announcements (e.g. "MSG-%m")
    announce_template: String,
}

impl IoHub {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: Box<dyn IoInstance>,
        server: Option<TcpServer>,
        rw_config: Option<RwConfig>,
        ro_server: Option<TcpServer>,
        monitor: Option<DeviceMonitor>,
        announce: bool,
        announce_template: String,
    ) -> Result<Self> {
        let mut signals = Signals::new([SIGINT, SIGTERM, SIGUSR1])?;
        let poll = Poll::new()?;

        poll.registry()
            .register(&mut signals, TOKEN_SIGNAL, Interest::READABLE)?;

        let mut io_hub = IoHub {
            poll,
            instances: HashMap::new(),
            device,
            server,
            rw_config,
            ro_server,
            monitor,
            signals,
            quit_requested: false,
            announce,
            device_write_blocked: false,
            pending_device_write: Vec::new(),
            last_device_status_msg: None,
            announce_template,
        };

        if let Some(s) = &mut io_hub.server {
            s.register(&mut io_hub.poll, TOKEN_SERVER)?;
        }

        if let Some(s) = &mut io_hub.ro_server {
            s.register(&mut io_hub.poll, TOKEN_RO_SERVER)?;
        }

        if let Some(m) = &mut io_hub.monitor {
            m.register(&mut io_hub.poll, TOKEN_MONITOR_SERVER)?;
        }

        // Publish the initial RW port to the port file (if configured).
        io_hub.write_port_file();

        Ok(io_hub)
    }

    fn next_free_token(&self) -> Token {
        let mut token_id = TOKEN_DYNAMIC_START.0;

        loop {
            let token = Token(token_id);
            if !self.instances.contains_key(&token) {
                return token;
            }
            token_id += 1;
        }
    }

    pub fn add(&mut self, mut instance: Box<dyn IoInstance>) -> Result<()> {
        let token = self.next_free_token();
        let addr = instance.addr_as_string();

        if let Err(e) = instance.connect(&mut self.poll, token) {
            error!("Hub({:?}): {} Failed to register {}", token, addr, e);
            return Err(e);
        }

        self.instances.insert(token, instance);

        info!("Hub({:?}): {} registered", token, addr);

        if self.announce
            && let Some(msg) = &self.last_device_status_msg
            && let Some(client) = self.instances.get_mut(&token)
        {
            client.write_announce(&self.announce_template, &client.addr_as_string(), msg);
        }

        Ok(())
    }

    /// Force-release: tear down the read-write server, disconnect all RW
    /// clients, and re-bind a fresh RW listener. With a random bind port (0)
    /// this yields a new port each time; with a fixed port the same port is
    /// re-bound. The new port is published to the port file. Read-only clients
    /// and the local console are untouched.
    fn force_release(&mut self) {
        // Only meaningful if a RW server is configured.
        if self.server.is_none() {
            info!("force_release: no read-write server configured, ignoring");
            return;
        }

        let bind_port = self.rw_config.as_ref().map(|c| c.bind_port).unwrap_or(0);

        // Bind the new listener first so a failure leaves the old one intact.
        // A random port (0) always yields a fresh free port. For a fixed port
        // the old listener still holds it, so we drop the old one first and
        // then rebind the same port.
        let mut new_server = match TcpServer::new(bind_port) {
            Ok(s) => s,
            Err(ref e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                if let Some(mut old) = self.server.take() {
                    let _ = old.deregister(&mut self.poll);
                }
                match TcpServer::new(bind_port) {
                    Ok(s) => s,
                    Err(e) => {
                        error!(
                            "force_release: failed to re-bind RW listener on port {}: {}",
                            bind_port, e
                        );
                        return;
                    }
                }
            }
            Err(e) => {
                error!(
                    "force_release: failed to bind new RW listener on port {}: {} — keeping existing server",
                    bind_port, e
                );
                return;
            }
        };

        if let Err(e) = new_server.register(&mut self.poll, TOKEN_SERVER) {
            error!("force_release: failed to register new RW listener: {}", e);
            return;
        }

        // Swap in the new listener; deregister and drop the old one (if the
        // fixed-port path above did not already take it).
        if let Some(mut old) = self.server.replace(new_server) {
            let _ = old.deregister(&mut self.poll);
        }

        // Disconnect all force-releasable clients (RW TCP clients).
        let tokens: Vec<Token> = self
            .instances
            .iter()
            .filter(|(_, c)| c.force_releasable())
            .map(|(&t, _)| t)
            .collect();
        for token in tokens {
            if let Some(mut client) = self.instances.remove(&token) {
                info!(
                    "force_release: disconnecting RW client {:?} {}",
                    token,
                    client.addr_as_string()
                );
                client.disconnect(&mut self.poll);
            }
        }

        // Publish the new port.
        self.write_port_file();

        if let Some(port) = self.server.as_ref().and_then(|s| s.port().ok()) {
            let msg = format!("Force-release: read-write server now at port {}", port);
            info!("{}", msg);
            self.all_clients_announce(&msg);
        }
    }

    /// Write the current read-write port (and pid) to the configured port file,
    /// atomically (write to a temp file in the same directory, then rename).
    /// No-op if there is no port file or no RW server.
    fn write_port_file(&self) {
        let Some(path) = self.rw_config.as_ref().and_then(|c| c.port_file.as_ref()) else {
            return;
        };
        let Some(port) = self.server.as_ref().and_then(|s| s.port().ok()) else {
            return;
        };

        let contents = format!("pid={}\nport={}\n", std::process::id(), port);
        let tmp = path.with_extension("tmp");

        let write_result = std::fs::File::create(&tmp)
            .and_then(|mut f| f.write_all(contents.as_bytes()).map(|_| f))
            .and_then(|mut f| f.flush())
            .and_then(|_| std::fs::rename(&tmp, path));

        match write_result {
            Ok(()) => info!("Wrote RW port file {}: port={}", path.display(), port),
            Err(e) => {
                error!("Failed to write RW port file {}: {}", path.display(), e);
                let _ = std::fs::remove_file(&tmp);
            }
        }
    }

    fn all_clients_str(&mut self, msg: String) {
        self.all_clients_announce(&msg);
    }

    fn all_clients_announce(&mut self, msg: &str) {
        info!("Announce: {}", msg.trim());
        if self.announce {
            for (_, client) in self.instances.iter_mut() {
                client.write_announce(&self.announce_template, &client.addr_as_string(), msg);
            }
        }
    }

    /// Forward client data to the device.  Sets `device_write_blocked` and
    /// registers WRITABLE interest when the device cannot accept the data.
    /// Unwritten bytes are saved in `pending_device_write` to avoid data loss.
    fn forward_to_device(&mut self, bytes: &[u8]) {
        if let Some(m) = &mut self.monitor {
            m.tx(bytes);
        }
        Self::try_device_write(
            &mut *self.device,
            &mut self.pending_device_write,
            &mut self.device_write_blocked,
            &mut self.poll,
            bytes,
        );
    }

    fn handle_read_result(&mut self, result: IoResult) {
        match result {
            IoResult::Data(bytes) => {
                self.forward_to_device(&bytes);
            }
            IoResult::Action(action) => {
                info!("Hub received action: {:?}", action);
                self.handle_action(action);
                info!(
                    "Hub handle_action returned, quit_requested = {}",
                    self.quit_requested
                );
            }
            IoResult::None => {}
        }
        trace!("handle_read_result returning");
    }

    fn handle_action(&mut self, action: Action) {
        match action {
            Action::Quit => {
                info!("Hub handling Quit action - setting quit_requested = true");
                self.quit_requested = true;
                info!("Hub quit_requested is now: {}", self.quit_requested);
            }
            Action::Send(bytes) => {
                info!("Hub handling Send action with {} bytes", bytes.len());
                self.forward_to_device(&bytes);
            }
            Action::FilterToggle(_) => {
                // Handled locally in Console, should not reach hub
                info!("Hub received FilterToggle (should be handled locally)");
            }
        }
        trace!("handle_action returning");
    }

    /// Try to write `bytes` to the device, buffering any remainder.
    /// Returns true if the device became blocked.
    fn try_device_write(
        device: &mut dyn IoInstance,
        pending: &mut Vec<u8>,
        blocked: &mut bool,
        poll: &mut Poll,
        bytes: &[u8],
    ) -> bool {
        let n = device.write_all(bytes);
        if n < bytes.len() {
            pending.extend_from_slice(&bytes[n..]);
            if !*blocked {
                info!("Device write blocked — enabling backpressure");
                *blocked = true;
                if let Err(e) = device.set_writable_interest(poll, true) {
                    error!("Failed to set writable interest: {}", e);
                }
            }
            true
        } else {
            false
        }
    }

    /// Read and forward data from a single client until WouldBlock or the
    /// device becomes write-blocked.
    fn drain_client(&mut self, token: Token) {
        trace!("drain_client({:?}): starting", token);
        loop {
            trace!(
                "drain_client({:?}): loop iteration, quit_requested={}",
                token, self.quit_requested
            );
            let (result, forwards_input) = match self.instances.get_mut(&token) {
                Some(client) if client.connected() => {
                    let forwards_input = client.forwards_input();
                    match client.read() {
                        Ok(IoResult::None) => {
                            trace!("drain_client({:?}): read returned None, breaking", token);
                            break;
                        }
                        Ok(result) => (result, forwards_input),
                        Err(_) => {
                            trace!("drain_client({:?}): read returned error, breaking", token);
                            break;
                        }
                    }
                }
                _ => {
                    trace!(
                        "drain_client({:?}): client not found or disconnected, breaking",
                        token
                    );
                    break;
                }
            };
            if forwards_input {
                trace!("drain_client({:?}): calling handle_read_result", token);
                self.handle_read_result(result);
                trace!("drain_client({:?}): handle_read_result returned", token);
            } else {
                // Read-only client: input is drained (to detect disconnect and
                // keep the socket buffer clear) but never forwarded.
                trace!("drain_client({:?}): read-only, discarding input", token);
            }
            if self.device_write_blocked {
                trace!("drain_client({:?}): device_write_blocked, breaking", token);
                break;
            }
            if self.quit_requested {
                trace!("drain_client({:?}): quit_requested, breaking", token);
                break;
            }
        }
        trace!("drain_client({:?}): exiting", token);
    }

    /// Drain pending client data after backpressure is lifted.
    ///
    /// With edge-triggered epoll we will not get new READABLE events for data
    /// that arrived while we were blocked, so we must explicitly read from
    /// every client once the device can accept data again.
    fn drain_pending_client_data(&mut self) {
        let tokens: Vec<Token> = self.instances.keys().copied().collect();
        for token in tokens {
            self.drain_client(token);
            if self.device_write_blocked {
                return;
            }
        }
    }

    pub fn handle_event(&mut self, event: &Event) -> Result<()> {
        let token_event = event.token();
        trace!("handle_event");

        if token_event == TOKEN_DEV {
            // Handle backpressure relief: device can accept writes again.
            if event.is_writable() && self.device_write_blocked {
                info!("Device write unblocked — flushing pending data");
                self.device_write_blocked = false;
                self.device.set_writable_interest(&mut self.poll, false)?;

                // Flush any bytes saved from a previous partial write.
                if !self.pending_device_write.is_empty() {
                    let pending = std::mem::take(&mut self.pending_device_write);
                    self.forward_to_device(&pending);
                }

                // Only drain clients if the pending flush didn't block again.
                if !self.device_write_blocked {
                    self.drain_pending_client_data();
                }
            }

            // Must loop until WouldBlock because mio uses edge-triggered epoll.
            // A single edge may signal multiple readable chunks.
            loop {
                match self.device.read() {
                    Ok(IoResult::Data(buf)) => {
                        if let Some(m) = &mut self.monitor {
                            m.rx(&buf);
                        }
                        for (_, client) in self.instances.iter_mut() {
                            if client.connected() {
                                client.write_all(&buf);
                            }
                        }
                    }
                    Ok(IoResult::None) => break,
                    Ok(IoResult::Action(_)) => {}
                    Err(e) => {
                        let msg = format!("{}: {}", self.device.addr_as_string(), e);
                        self.last_device_status_msg = Some(msg.clone());
                        self.all_clients_str(msg);
                        break;
                    }
                }
            }
        } else if token_event == TOKEN_SERVER {
            // Must loop until WouldBlock because mio uses edge-triggered epoll.
            // A single edge may signal multiple pending connections.
            let mut new_clients = Vec::new();
            if let Some(s) = &mut self.server {
                while let Some(c) = s.accept() {
                    new_clients.push(c);
                }
            }
            for c in new_clients {
                self.add(c)?;
            }
        } else if token_event == TOKEN_RO_SERVER {
            // Read-only listen-in clients. Same accept path as the RW server;
            // the accepted clients are tagged read-only so their input is
            // discarded and they are immune to force-release.
            let mut new_clients = Vec::new();
            if let Some(s) = &mut self.ro_server {
                while let Some(c) = s.accept() {
                    new_clients.push(c);
                }
            }
            for c in new_clients {
                self.add(c)?;
            }
        } else if token_event == TOKEN_MONITOR_SERVER {
            if let Some(m) = &mut self.monitor {
                m.accept(&mut self.poll)?;
            }
        } else if token_event == TOKEN_SIGNAL {
            for signal in self.signals.pending() {
                match signal {
                    SIGUSR1 => {
                        info!("Received SIGUSR1, performing force-release");
                        self.force_release();
                    }
                    _ => {
                        info!("Received signal {}, initiating graceful shutdown", signal);
                        self.quit_requested = true;
                    }
                }
            }
        } else if self.instances.contains_key(&token_event) {
            // NOTICE: The 'console' is also a client
            if !self.device_write_blocked {
                self.drain_client(token_event);
            }
        } else {
            // With edge-triggered epoll, stale events can arrive for tokens that were
            // removed earlier in the same event batch. This is expected and harmless.
            trace!("Ignoring event for unknown token: {}", token_event.0);
        }

        // Clean up all instances not connected ///////////////////////////////
        let mut disconnected_tokens = Vec::new();
        for (&t, client) in self.instances.iter_mut() {
            if !client.connected() {
                let addr = client.addr_as_string();
                info!("Hub({:?}): {}: disconnect()", t, addr);
                client.disconnect(&mut self.poll);
                disconnected_tokens.push(t);
            }
        }

        for t in disconnected_tokens {
            info!("Hub({:?}): Remove", t);
            self.instances.remove(&t);
        }

        Ok(())
    }

    pub fn is_quit_requested(&self) -> bool {
        self.quit_requested
    }

    pub fn run(&mut self) -> std::io::Result<()> {
        let mut events = Events::with_capacity(128);
        let tick = Duration::from_millis(100);
        let mut last_tick = Instant::now();

        loop {
            if self.device.disconnect_needed() {
                self.device.disconnect(&mut self.poll);
                // Keep device_write_blocked set — clients stay blocked until
                // the device reconnects and can accept data again.
                // Discard pending data — the device connection is gone.
                self.pending_device_write.clear();
            }

            // This will ensure devices are re-connected. If a device cannot be connected right
            // away, then print a message to warn the user that nothing is connected.
            // If a device is dis-connected at a later point, then a message will be printed when
            // disconnected.
            // Always print once connected.
            if !self.device.connected() {
                let status_msg = match self.device.connect(&mut self.poll, TOKEN_DEV) {
                    Ok(()) => {
                        self.device_write_blocked = false;
                        self.device.connected_announcement()
                    }

                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        // Connection in progress - silently wait
                        None
                    }

                    Err(e) => Some(format!("{}: {}", self.device.addr_as_string(), e)),
                };

                if let Some(msg) = status_msg
                    && Some(&msg) != self.last_device_status_msg.as_ref()
                {
                    self.last_device_status_msg = Some(msg.clone());
                    self.all_clients_announce(&msg);
                }
            }

            match self.poll.poll(&mut events, Some(tick)) {
                Ok(()) => {}
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {
                    // EINTR - signal received, loop will continue and signal
                    // will be processed on next poll iteration
                }
                Err(e) => return Err(e),
            }

            for event in events.iter() {
                self.handle_event(event)?;
            }
            trace!("Finished processing {} events", events.iter().count());

            // Process timeouts for all instances (e.g., keybind timeouts in Console)
            let results: Vec<_> = self
                .instances
                .values_mut()
                .filter_map(|c| c.tick().ok())
                .collect();
            for result in results {
                self.handle_read_result(result);
            }
            trace!("Finished processing timeouts");

            // Check if quit was requested
            trace!("Checking quit_requested: {}", self.quit_requested);
            if self.quit_requested {
                info!("Quit requested - exiting hub.run()");
                return Ok(());
            }

            let now = Instant::now();
            while now.duration_since(last_tick) >= tick {
                last_tick = now;
            }
        }
    }
}

impl Drop for IoHub {
    fn drop(&mut self) {
        // Remove the RW port file on shutdown so the reservation system does
        // not attempt to connect to a port that is no longer served.
        if let Some(path) = self.rw_config.as_ref().and_then(|c| c.port_file.as_ref()) {
            match std::fs::remove_file(path) {
                Ok(()) => info!("Removed RW port file {}", path.display()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => error!("Failed to remove RW port file {}: {}", path.display(), e),
            }
        }
    }
}
