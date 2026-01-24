use log::info;
use mio::{Interest, Poll, Token};
use mio_serial::{SerialPortBuilderExt, SerialStream};
use std::io::{Error, ErrorKind, Read, Result, Write};
use std::time::{Duration, Instant};

use crate::traits::{IoInstance, IoResult};

pub struct Connection {
    stream: SerialStream,
    connected_at: Instant,

    // Some USB devices sends a lot of old charters at connect - this is used to discard those.
    quarantine: bool,
}

pub struct SerialDevice {
    path: String,
    baudrate: u32,
    zombie: bool,
    connection: Option<Connection>,
}

impl SerialDevice {
    pub fn new(path: String, baudrate: u32) -> Result<Self> {
        Ok(SerialDevice {
            path,
            baudrate,
            zombie: false,
            connection: None,
        })
    }

    fn err_handle_zombie(&mut self, method: &'static str, err: Error) -> Result<IoResult> {
        info!("UART-Device/{}: {} -> zombie", method, err);
        self.zombie = true;
        Err(err)
    }
}

impl IoInstance for SerialDevice {
    fn connect(&mut self, poll: &mut Poll, token: Token) -> Result<()> {
        let mut serial = mio_serial::new(self.path.clone(), self.baudrate)
            .timeout(Duration::from_millis(250))
            .open_native_async()?;
        serial.set_exclusive(true)?;

        let mut c = Connection {
            stream: serial,
            connected_at: Instant::now(),
            quarantine: true,
        };

        poll.registry()
            .register(&mut c.stream, token, Interest::READABLE)?;

        // Must be done after register(), as the connection must be closed by RAII if register
        // fails
        self.connection = Some(c);

        Ok(())
    }

    fn connected(&self) -> bool {
        self.connection.is_some()
    }

    fn disconnect_needed(&self) -> bool {
        self.zombie
    }

    fn disconnect(&mut self, poll: &mut Poll) {
        if let Some(c) = &mut self.connection {
            poll.registry()
                .deregister(&mut c.stream)
                .expect("BUG: Deregister failed!");
        }
        self.zombie = false;
        self.connection = None;
    }

    fn read(&mut self) -> Result<IoResult> {
        let mut tmp = [0u8; 1024];

        if let Some(c) = &mut self.connection {
            match c.stream.read(&mut tmp) {
                Ok(0) => {
                    info!("uart EOF");
                    self.zombie = true;
                    Err(Error::other("Device disconnected".to_string()))
                }

                Ok(n) => {
                    if c.quarantine {
                        let now = Instant::now();
                        if now - c.connected_at > Duration::from_millis(10) {
                            c.quarantine = false;
                        }
                    }

                    if c.quarantine {
                        info!("Skipping {} bytes due to quarantine", n);
                        Ok(IoResult::None)
                    } else {
                        Ok(IoResult::Data(tmp[..n].to_vec()))
                    }
                }

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
        if let Some(c) = &mut self.connection {
            match c.stream.write(buf) {
                Ok(n) => Ok(IoResult::Data(buf[..n].to_vec())),

                Err(e) => self.err_handle_zombie("write", e),
            }
        } else {
            Err(Error::other("Device not connected".to_string()))
        }
    }

    fn flush(&mut self) {
        if let Some(c) = &mut self.connection
            && let Err(e) = c.stream.flush()
        {
            let _ = self.err_handle_zombie("flush", e);
        }
    }

    fn addr_as_string(&self) -> String {
        self.path.clone()
    }
}
