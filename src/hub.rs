use log::{error, info, trace};
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

    fn handle_read_result(&mut self, result: IoResult) {
        match result {
            IoResult::Data(bytes) => {
                // TODO, handle write error
                self.device.write_all(&bytes);
            }
            IoResult::Action(action) => {
                self.handle_action(action);
            }
            IoResult::None => {}
        }
    }

    fn handle_action(&mut self, action: Action) {
        match action {
            Action::Quit => {
                self.quit_requested = true;
            }
            Action::Send(bytes) => {
                // TODO, handle write error
                self.device.write_all(&bytes);
            }
            Action::FilterToggle(_) => {
                // Handled locally in Console, should not reach hub
            }
        }
    }

    pub fn handle_event(&mut self, token_event: Token) -> Result<()> {
        trace!("handle_event");

        if token_event == TOKEN_DEV {
            // Must loop until WouldBlock because mio uses edge-triggered epoll.
            // A single edge may signal multiple readable chunks.
            loop {
                match self.device.read() {
                    Ok(IoResult::Data(buf)) => {
                        for (_, client) in self.instances.iter_mut() {
                            client.write_all(&buf);
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
        } else if let Some(client) = self.instances.get_mut(&token_event) {
            // NOTICE: The 'console' is also a client
            // Must loop until WouldBlock because mio uses edge-triggered epoll.
            let mut results = Vec::new();
            loop {
                match client.read() {
                    Ok(IoResult::None) => break,
                    Ok(result) => results.push(result),
                    Err(_) => break,
                }
            }
            for result in results {
                self.handle_read_result(result);
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
                self.handle_event(event.token())?;
            }

            // Process timeouts for all instances (e.g., keybind timeouts in Console)
            let results: Vec<_> = self
                .instances
                .values_mut()
                .filter_map(|c| c.tick().ok())
                .collect();
            for result in results {
                self.handle_read_result(result);
            }

            // Check if quit was requested
            if self.quit_requested {
                return Ok(());
            }

            let now = Instant::now();
            while now.duration_since(last_tick) >= tick {
                last_tick = now;
            }
        }
    }
}
