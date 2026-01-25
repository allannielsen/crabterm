use log::info;
use mio::{Interest, Poll, Token, net::TcpStream};
use std::io::{Error, ErrorKind, Read, Result, Write};
use std::net::SocketAddr;

use crate::traits::{IoInstance, IoResult};

pub struct TcpDevice {
    stream: Option<TcpStream>,
    addr: SocketAddr,
    zombie: bool,
    /// True while connection is in progress (not yet verified)
    connecting: bool,
    /// Token used for poll registration (needed for re-registration)
    token: Option<Token>,
}

impl TcpDevice {
    pub fn new(addr: SocketAddr) -> Result<Self> {
        Ok(TcpDevice {
            stream: None,
            addr,
            zombie: false,
            connecting: false,
            token: None,
        })
    }

    fn err_handle_zombie(&mut self, method: &'static str, err: Error) -> Result<IoResult> {
        info!("TCP-Device/{}: {} -> zombie", method, err);
        self.zombie = true;
        Err(err)
    }
}

impl IoInstance for TcpDevice {
    fn connect(&mut self, poll: &mut Poll, token: Token) -> Result<()> {
        // Already connecting - check if connection completed
        if self.connecting
            && let Some(s) = &mut self.stream
        {
            if let Ok(Some(err)) = s.take_error() {
                // Connection failed
                info!("TCP-Device/connect: {} -> zombie", err);
                self.zombie = true;
                self.connecting = false;
                return Err(err);
            }
            // Connection succeeded - re-register for READABLE only (not WRITABLE)
            poll.registry().reregister(s, token, Interest::READABLE)?;
            info!("TCP-Device/{}: Connection verified", self.addr_as_string());
            self.connecting = false;
            return Ok(());
        }

        // Already connected
        if self.stream.is_some() {
            return Ok(());
        }

        info!("TCP-Device/{}: Try connect", self.addr_as_string());
        let mut s = TcpStream::connect(self.addr)?;

        // Register for WRITABLE to detect connection completion, plus READABLE for data
        poll.registry()
            .register(&mut s, token, Interest::READABLE | Interest::WRITABLE)?;

        self.stream = Some(s);
        self.connecting = true; // Connection in progress, not yet verified
        self.token = Some(token);

        // Return WouldBlock to indicate connection is in progress
        Err(Error::new(ErrorKind::WouldBlock, "Connection in progress"))
    }

    fn addr_as_string(&self) -> String {
        format!("TCP-Device:{}", self.addr)
    }

    fn connected(&self) -> bool {
        self.stream.is_some() && !self.connecting
    }

    fn disconnect_needed(&self) -> bool {
        self.zombie
    }

    fn disconnect(&mut self, poll: &mut Poll) {
        if let Some(s) = &mut self.stream {
            poll.registry()
                .deregister(s)
                .expect("BUG: Deregister failed!");
        }
        self.zombie = false;
        self.connecting = false;
        self.stream = None;
    }

    fn read(&mut self) -> Result<IoResult> {
        let mut tmp = [0u8; 1024];

        // If still connecting, wait for connect() to verify
        if self.connecting {
            return Ok(IoResult::None);
        }

        if let Some(s) = &mut self.stream {
            match s.read(&mut tmp) {
                Ok(0) => {
                    info!("tcp device EOF");
                    self.zombie = true;
                    Err(Error::other("Disconnected".to_string()))
                }

                Ok(n) => Ok(IoResult::Data(tmp[..n].to_vec())),

                Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                    // Not ready yet — ignore and wait for next event
                    Ok(IoResult::None)
                }

                Err(e) => self.err_handle_zombie("read", e),
            }
        } else {
            Err(Error::other("Device not connected".to_string()))
        }
    }

    fn write(&mut self, buf: &[u8]) -> Result<IoResult> {
        if let Some(s) = &mut self.stream {
            match s.write(buf) {
                Ok(n) => Ok(IoResult::Data(buf[..n].to_vec())),

                Err(e) => self.err_handle_zombie("write", e),
            }
        } else {
            Err(Error::other("Device not connected".to_string()))
        }
    }

    fn flush(&mut self) {
        if let Some(s) = &mut self.stream
            && let Err(e) = s.flush()
        {
            let _ = self.err_handle_zombie("flush", e);
        }
    }
}
