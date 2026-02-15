use mio::{Poll, Token};
use std::io::Result;

use crate::keybind::Action;

pub const TOKEN_DEV: Token = Token(0);
pub const TOKEN_SERVER: Token = Token(1);
pub const TOKEN_SIGNAL: Token = Token(2);
pub const TOKEN_DYNAMIC_START: Token = Token(3);

/// Result of an I/O operation
#[derive(Debug)]
pub enum IoResult {
    /// Data
    Data(Vec<u8>),
    /// Action to be performed by the hub
    Action(Action),
    /// Nothing
    None,
}

pub trait IoInstance {
    fn connect(&mut self, poll: &mut Poll, token: Token) -> Result<()>;
    fn connected(&self) -> bool;

    fn disconnect_needed(&self) -> bool {
        false
    }

    fn disconnect(&mut self, poll: &mut Poll);

    fn read(&mut self) -> Result<IoResult>;
    fn write(&mut self, buf: &[u8]) -> Result<IoResult>;
    fn flush(&mut self);

    fn addr_as_string(&self) -> String;

    /// Called periodically to handle timeouts etc.
    fn tick(&mut self) -> Result<IoResult> {
        Ok(IoResult::None)
    }

    /// Write all bytes. Returns the number of bytes actually written.
    /// A short write indicates backpressure (e.g. WouldBlock).
    fn write_all(&mut self, buf: &[u8]) -> usize {
        let mut written = 0;
        while written < buf.len() {
            match self.write(&buf[written..]) {
                Ok(IoResult::Data(d)) if !d.is_empty() => written += d.len(),
                // Break on empty data (socket can't accept more) or error
                // to avoid spinning when no progress is made.
                _ => break,
            }
        }
        self.flush();
        written
    }

    /// Request WRITABLE interest from the poll loop so that the caller is
    /// notified when the underlying socket can accept data again.
    /// Default is a no-op for devices that don't support this.
    fn set_writable_interest(&mut self, _poll: &mut Poll, _writable: bool) -> Result<()> {
        Ok(())
    }
}
