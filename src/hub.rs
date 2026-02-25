use log::{error, info, trace};
use mio::event::Event;
use mio::{Events, Interest, Poll, Token};
use signal_hook::consts::signal::{SIGINT, SIGTERM};
use signal_hook_mio::v1_0::Signals;
use std::collections::HashMap;
use std::io::Result;
use std::time::{Duration, Instant};

use crate::io::TcpServer;
use crate::keybind::Action;
use crate::traits::{IoInstance, IoResult, TOKEN_DEV, TOKEN_DYNAMIC_START, TOKEN_SERVER, TOKEN_SIGNAL};

pub struct IoHub {
    poll: Poll,
    instances: HashMap<Token, Box<dyn IoInstance>>,

    // The device is special, which is why we do not want it as part of the
    // instances (despite it is has a compatible type).
    device: Box<dyn IoInstance>,

    server: Option<TcpServer>,

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
}

impl IoHub {
    pub fn new(device: Box<dyn IoInstance>, server: Option<TcpServer>, announce: bool) -> Result<Self> {
        let mut signals = Signals::new([SIGINT, SIGTERM])?;
        let poll = Poll::new()?;

        poll.registry()
            .register(&mut signals, TOKEN_SIGNAL, Interest::READABLE)?;

        let mut io_hub = IoHub {
            poll,
            instances: HashMap::new(),
            device,
            server,
            signals,
            quit_requested: false,
            announce,
            device_write_blocked: false,
            pending_device_write: Vec::new(),
        };

        if let Some(s) = &mut io_hub.server {
            s.register(&mut io_hub.poll, TOKEN_SERVER)?;
        }

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
        Ok(())
    }

    fn all_clients_str(&mut self, msg: String) {
        info!("Announce: {}", msg.trim());
        if self.announce {
            for (_, client) in self.instances.iter_mut() {
                client.write_all(msg.as_bytes());
            }
        }
    }

    /// Forward client data to the device.  Sets `device_write_blocked` and
    /// registers WRITABLE interest when the device cannot accept the data.
    /// Unwritten bytes are saved in `pending_device_write` to avoid data loss.
    fn forward_to_device(&mut self, bytes: &[u8]) {
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
                info!("Hub handle_action returned, quit_requested = {}", self.quit_requested);
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
            trace!("drain_client({:?}): loop iteration, quit_requested={}", token, self.quit_requested);
            let result = match self.instances.get_mut(&token) {
                Some(client) if client.connected() => match client.read() {
                    Ok(IoResult::None) => {
                        trace!("drain_client({:?}): read returned None, breaking", token);
                        break;
                    }
                    Ok(result) => result,
                    Err(_) => {
                        trace!("drain_client({:?}): read returned error, breaking", token);
                        break;
                    }
                },
                _ => {
                    trace!("drain_client({:?}): client not found or disconnected, breaking", token);
                    break;
                }
            };
            trace!("drain_client({:?}): calling handle_read_result", token);
            self.handle_read_result(result);
            trace!("drain_client({:?}): handle_read_result returned", token);
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
                        for (_, client) in self.instances.iter_mut() {
                            if client.connected() {
                                client.write_all(&buf);
                            }
                        }
                    }
                    Ok(IoResult::None) => break,
                    Ok(IoResult::Action(_)) => {}
                    Err(e) => {
                        self.all_clients_str(format!(
                            "\n\rInfo: {}: {}\n\r",
                            self.device.addr_as_string(),
                            e
                        ));
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
        } else if token_event == TOKEN_SIGNAL {
            for signal in self.signals.pending() {
                info!("Received signal {}, initiating graceful shutdown", signal);
                self.quit_requested = true;
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
        let mut device_connect_warn_first_only = true;
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
                match self.device.connect(&mut self.poll, TOKEN_DEV) {
                    Ok(()) => {
                        device_connect_warn_first_only = false;
                        self.device_write_blocked = false;
                        self.all_clients_str(format!(
                            "Info: {}: Connected\n\r",
                            self.device.addr_as_string()
                        ));
                    }

                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        // Connection in progress - silently wait
                    }

                    Err(e) => {
                        if device_connect_warn_first_only {
                            device_connect_warn_first_only = false;
                            self.all_clients_str(format!(
                                "Error: {}: {}\n\r",
                                self.device.addr_as_string(),
                                e
                            ));
                        }
                    }
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
