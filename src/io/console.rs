use mio::unix::SourceFd;
use mio::{Interest, Poll, Token};
use std::io::{ErrorKind, Read, Result, Write};
use std::os::unix::io::AsRawFd;

use crate::term::{disable_raw_mode, enable_raw_mode};

use crate::traits::IoInstance;

pub struct Console {
    fd_in: SourceFd<'static>,
}

impl Console {
    pub fn new() -> Result<Self> {
        // stdin is a global and its FD is valid for the entire program
        let fd = std::io::stdin().as_raw_fd();

        enable_raw_mode()?;

        let fd_ref: &'static i32 = Box::leak(Box::new(fd)); // convert to 'static lifetime

        Ok(Console {
            fd_in: SourceFd(fd_ref),
        })
    }
}

impl IoInstance for Console {
    fn connect(&mut self, poll: &mut Poll, token: Token) -> Result<()> {
        poll.registry()
            .register(&mut self.fd_in, token, Interest::READABLE)
    }

    fn addr_as_string(&self) -> String {
        "Console".to_owned()
    }

    fn connected(&self) -> bool {
        true
    }

    fn disconnect(&mut self, poll: &mut Poll) {
        // TODO, panic on error?
        let _ = poll.registry().deregister(&mut self.fd_in);
    }

    fn read(&mut self, buf: &mut Vec<u8>) -> Result<usize> {
        let mut tmp = [0u8; 1024];

        match std::io::stdin().read(&mut tmp) {
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                Ok(n)
            }

            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                // Not ready yet — ignore and wait for next event
                Ok(0)
            }

            Err(e) => Err(e),
        }
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        std::io::stdout().write(buf)
    }

    fn flush(&mut self) {
        // TODO, error handle
        let _ = std::io::stdout().flush();
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}
