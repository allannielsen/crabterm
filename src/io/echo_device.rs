use log::info;
use mio::unix::pipe::{Receiver, Sender};
use mio::{Interest, Poll, Token};
use std::io::{ErrorKind, Read, Result, Write};

use crate::traits::{IoInstance, IoResult};

pub struct EchoDevice {
    sender: Option<Sender>,
    receiver: Option<Receiver>,
}

impl EchoDevice {
    pub fn new() -> Result<Self> {
        Ok(EchoDevice {
            sender: None,
            receiver: None,
        })
    }
}

impl IoInstance for EchoDevice {
    fn connect(&mut self, poll: &mut Poll, token: Token) -> Result<()> {
        let (sender, mut receiver) = mio::unix::pipe::new()?;

        poll.registry()
            .register(&mut receiver, token, Interest::READABLE)?;

        self.sender = Some(sender);
        self.receiver = Some(receiver);

        info!("EchoDevice connected");
        Ok(())
    }

    fn addr_as_string(&self) -> String {
        "Echo".to_string()
    }

    fn connected(&self) -> bool {
        self.receiver.is_some()
    }

    fn disconnect(&mut self, poll: &mut Poll) {
        if let Some(r) = &mut self.receiver {
            poll.registry()
                .deregister(r)
                .expect("BUG: Deregister failed!");
        }
        self.sender = None;
        self.receiver = None;
    }

    fn read(&mut self) -> Result<IoResult> {
        let mut tmp = [0u8; 1024];

        if let Some(r) = &mut self.receiver {
            match r.read(&mut tmp) {
                Ok(0) => Ok(IoResult::None),

                Ok(n) => Ok(IoResult::Data(tmp[..n].to_vec())),

                Err(ref e) if e.kind() == ErrorKind::WouldBlock => Ok(IoResult::None),

                Err(e) => Err(e),
            }
        } else {
            Ok(IoResult::None)
        }
    }

    fn write(&mut self, buf: &[u8]) -> Result<IoResult> {
        if let Some(s) = &mut self.sender {
            match s.write(buf) {
                Ok(n) => Ok(IoResult::Data(buf[..n].to_vec())),
                Err(e) => Err(e),
            }
        } else {
            Ok(IoResult::None)
        }
    }

    fn flush(&mut self) {
        if let Some(s) = &mut self.sender {
            let _ = s.flush();
        }
    }
}
