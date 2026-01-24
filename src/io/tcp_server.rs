use crate::traits::{IoInstance, IoResult};
use log::{error, info};
use mio::net::{TcpListener, TcpStream};
use mio::{Interest, Poll, Token};
use std::io::{ErrorKind, Read, Result, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr};

pub struct TcpServer {
    listener: TcpListener,
}

impl TcpServer {
    pub fn new(port: u16) -> Result<Self> {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);
        let listener = TcpListener::bind(addr)?;

        Ok(TcpServer { listener })
    }

    pub fn register(&mut self, poll: &mut Poll, token: Token) -> Result<()> {
        poll.registry()
            .register(&mut self.listener, token, Interest::READABLE)
    }

    pub fn accept(&mut self) -> Option<Box<dyn IoInstance>> {
        match self.listener.accept() {
            Ok((stream, addr)) => {
                info!("TcpClient:{} New client connected", addr);
                let client = TcpClient {
                    stream,
                    addr,
                    connected: true,
                };
                Some(Box::new(client))
            }

            Err(ref e) if e.kind() == ErrorKind::WouldBlock => None,

            Err(e) => {
                error!("Accept error: {}", e);
                None
            }
        }
    }
}

pub struct TcpClient {
    stream: TcpStream,
    addr: SocketAddr,
    connected: bool,
}

impl TcpClient {
    fn close(&mut self) {
        self.connected = false;
        if let Err(e) = self.stream.shutdown(Shutdown::Both) {
            error!("TcpClient:{} Shutdown error: {}", self.addr, e);
        }
    }
}

impl IoInstance for TcpClient {
    fn connect(&mut self, poll: &mut Poll, token: Token) -> Result<()> {
        poll.registry()
            .register(&mut self.stream, token, Interest::READABLE)
            .map_err(|e| {
                error!("TcpClient:{} Register error: {}", self.addr, e);
                e
            })
    }

    fn connected(&self) -> bool {
        self.connected
    }

    fn addr_as_string(&self) -> String {
        format!("TCP-Client:{}", self.addr)
    }

    fn disconnect(&mut self, poll: &mut Poll) {
        self.close();

        if let Err(e) = poll.registry().deregister(&mut self.stream) {
            error!("TcpClient:{} Deregister error: {}", self.addr, e);
        }
    }

    fn read(&mut self) -> Result<IoResult> {
        let mut tmp = [0u8; 1024];

        match self.stream.read(&mut tmp) {
            Ok(0) => Ok(IoResult::None),

            Ok(n) => Ok(IoResult::Data(tmp[..n].to_vec())),

            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                // Not ready yet — ignore and wait for next event
                Ok(IoResult::None)
            }

            Err(e) => {
                info!("TcpClient:{} Read error: {}", self.addr, e);
                self.close();
                Err(e)
            }
        }
    }

    fn write(&mut self, buf: &[u8]) -> Result<IoResult> {
        match self.stream.write(buf) {
            Ok(n) => Ok(IoResult::Data(buf[..n].to_vec())),
            Err(e) => {
                info!("TcpClient:{} Write error: {}", self.addr, e);
                self.close();
                Err(e)
            }
        }
    }

    fn flush(&mut self) {
        if let Err(e) = self.stream.flush() {
            info!("TcpClient:{} Flush error: {}", self.addr, e);
            self.close();
        }
    }
}

impl Drop for TcpClient {
    fn drop(&mut self) {
        info!("TcpClient:{} dropped", self.addr);
    }
}
