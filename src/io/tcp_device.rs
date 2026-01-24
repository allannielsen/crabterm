use log::info;
use mio::{Interest, Poll, Token, net::TcpStream};
use std::io::{Error, ErrorKind, Read, Result, Write};
use std::net::SocketAddr;

use crate::traits::IoInstance;

pub struct TcpDevice {
    stream: Option<TcpStream>,
    addr: SocketAddr,
    zombie: bool,
}

impl TcpDevice {
    pub fn new(addr: SocketAddr) -> Result<Self> {
        Ok(TcpDevice {
            stream: None,
            addr,
            zombie: false,
        })
    }

    fn err_handle_zombie(&mut self, method: &'static str, err: Error) -> Result<usize> {
        info!("TCP-Device/{}: {} -> zombie", method, err);
        self.zombie = true;
        Err(err)
    }
}

impl IoInstance for TcpDevice {
    fn connect(&mut self, poll: &mut Poll, token: Token) -> Result<()> {
        let mut s = TcpStream::connect(self.addr)?;

        poll.registry()
            .register(&mut s, token, Interest::READABLE)?;

        self.stream = Some(s);

        Ok(())
    }

    fn addr_as_string(&self) -> String {
        format!("TCP-Device:{}", self.addr)
    }

    fn connected(&self) -> bool {
        self.stream.is_some()
    }

    fn disconnect(&mut self, poll: &mut Poll) {
        if let Some(s) = &mut self.stream {
            poll.registry()
                .deregister(s)
                .expect("BUG: Deregister failed!");
        }
        self.zombie = false;
        self.stream = None;
    }

    fn read(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        let mut tmp = [0u8; 1024];

        if let Some(s) = &mut self.stream {
            match s.read(&mut tmp) {
                Ok(0) => {
                    info!("tcp device EOF");
                    self.zombie = true;
                    Err(Error::other("Disconnected".to_string()))
                }

                Ok(n) => {
                    buf.extend_from_slice(&tmp[..n]);
                    Ok(n)
                }

                Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                    // Not ready yet — ignore and wait for next event
                    Ok(0)
                }

                Err(e) => self.err_handle_zombie("read", e),
            }
        } else {
            Err(Error::other("Device not connected".to_string()))
        }
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        if let Some(s) = &mut self.stream {
            match s.write(buf) {
                Ok(n) => Ok(n),

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
