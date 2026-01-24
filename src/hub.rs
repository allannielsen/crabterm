use log::{error, info, trace};
use mio::{Events, Poll, Token};
use std::collections::HashMap;
use std::io::Result;
use std::time::{Duration, Instant};

use crate::io::TcpServer;
use crate::traits::{IoInstance, TOKEN_DEV, TOKEN_DYNAMIC_START, TOKEN_SERVER};

pub struct IoHub {
    poll: Poll,
    instances: HashMap<Token, Box<dyn IoInstance>>,

    // The device is special, which is why we do not want it as part of the
    // instances (despite it is has a compatible type).
    device: Box<dyn IoInstance>,

    server: Option<TcpServer>,
}

impl IoHub {
    pub fn new(device: Box<dyn IoInstance>, server: Option<TcpServer>) -> Result<Self> {
        let mut io_hub = IoHub {
            poll: Poll::new()?,
            instances: HashMap::new(),
            device,
            server,
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
        for (_, client) in self.instances.iter_mut() {
            client.write_all(msg.as_bytes());
        }
    }

    pub fn handle_event(&mut self, token_event: Token) -> Result<()> {
        let mut buf = Vec::new();

        trace!("handle_event");

        if token_event == TOKEN_DEV {
            match self.device.read(&mut buf) {
                Ok(0) => {}

                Ok(_) => {
                    // Broadcast to all clients
                    for (_, client) in self.instances.iter_mut() {
                        client.write_all(&buf);
                    }
                }

                Err(e) => {
                    self.all_clients_str(format!(
                        "\n\rInfo: {}: {}\n\r",
                        self.device.addr_as_string(),
                        e
                    ));
                }
            }
        } else if token_event == TOKEN_SERVER {
            if let Some(s) = &mut self.server
                && let Some(c) = s.accept()
            {
                self.add(c)?;
            }
        } else if let Some(client) = self.instances.get_mut(&token_event) {
            // NOTICE: The 'console' is also a client
            match client.read(&mut buf) {
                Ok(0) => {}

                Ok(_) => {
                    // TODO, handle write error
                    self.device.write_all(&buf);
                }

                Err(_) => {}
            }
        } else {
            panic!("Unexpected token became ready: {}", token_event.0);
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

            self.poll.poll(&mut events, Some(tick))?;

            for event in events.iter() {
                self.handle_event(event.token())?;
            }

            let now = Instant::now();
            while now.duration_since(last_tick) >= tick {
                last_tick = now;
            }
        }
    }
}
