use mio::{Poll, Token};
use std::io::Result;

pub const TOKEN_DEV: Token = Token(0);
pub const TOKEN_SERVER: Token = Token(1);
pub const TOKEN_DYNAMIC_START: Token = Token(2);

pub trait IoInstance {
    fn connect(&mut self, poll: &mut Poll, token: Token) -> Result<()>;
    fn connected(&self) -> bool;

    fn disconnect_needed(&self) -> bool {
        false
    }

    fn disconnect(&mut self, poll: &mut Poll);

    // fn io_type(&self) -> IoType;
    fn read(&mut self, buf: &mut Vec<u8>) -> Result<usize>;
    fn write(&mut self, buf: &[u8]) -> Result<usize>;
    fn flush(&mut self);

    fn addr_as_string(&self) -> String;

    fn write_all(&mut self, buf: &[u8]) {
        let mut written = 0;
        'outer: while written < buf.len() {
            match self.write(&buf[written..]) {
                Ok(0) => {}
                Ok(n) => {
                    written += n;
                }
                Err(_) => {
                    break 'outer;
                }
            }
        }
        self.flush()
    }
}
