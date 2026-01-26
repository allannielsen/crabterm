use mio::unix::SourceFd;
use mio::{Interest, Poll, Token};
use std::io::{ErrorKind, Read, Result, Write};
use std::os::unix::io::AsRawFd;

use crate::iofilter::FilterChain;
use crate::keybind::action::Action;
use crate::keybind::{KeybindConfig, KeybindProcessor, KeybindResult};
use crate::term::{disable_raw_mode, enable_raw_mode};
use crate::traits::{IoInstance, IoResult};

pub struct Console {
    fd_in: SourceFd<'static>,
    keybind_processor: KeybindProcessor,
    pending_results: Vec<KeybindResult>,
    filter_chain: FilterChain,
}

impl Console {
    pub fn new(keybind_config: KeybindConfig, filter_chain: FilterChain) -> Result<Self> {
        // stdin is a global and its FD is valid for the entire program
        let fd = std::io::stdin().as_raw_fd();

        enable_raw_mode()?;

        let fd_ref: &'static i32 = Box::leak(Box::new(fd)); // convert to 'static lifetime

        Ok(Console {
            fd_in: SourceFd(fd_ref),
            keybind_processor: KeybindProcessor::new(keybind_config),
            pending_results: Vec::new(),
            filter_chain,
        })
    }

    fn keybind_result_to_read_result(&mut self, result: KeybindResult) -> Option<IoResult> {
        match result {
            KeybindResult::Passthrough(bytes) => {
                let filtered = self.filter_chain.filter_in(&bytes);
                Some(IoResult::Data(filtered))
            }
            KeybindResult::Action(Action::FilterToggle(name)) => {
                self.filter_chain.toggle(&name);
                None
            }
            KeybindResult::Action(action) => Some(IoResult::Action(action)),
            KeybindResult::Consumed => None,
        }
    }

    fn apply_filter(&mut self, buf: &[u8]) -> Vec<u8> {
        self.filter_chain.filter_out(buf)
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

    fn read(&mut self) -> Result<IoResult> {
        // First, check if we have pending results from previous processing
        if let Some(result) = self.pending_results.pop()
            && let Some(read_result) = self.keybind_result_to_read_result(result)
        {
            return Ok(read_result);
        }

        let mut tmp = [0u8; 1024];

        match std::io::stdin().read(&mut tmp) {
            Ok(0) => Ok(IoResult::None),

            Ok(n) => {
                // Process through keybind processor
                let results = self.keybind_processor.process(&tmp[..n]);

                // Store results in reverse order so we can pop from the end
                for result in results.into_iter().rev() {
                    self.pending_results.push(result);
                }

                // Return the first result
                if let Some(result) = self.pending_results.pop()
                    && let Some(read_result) = self.keybind_result_to_read_result(result)
                {
                    return Ok(read_result);
                }

                Ok(IoResult::None)
            }

            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                // Not ready yet — ignore and wait for next event
                Ok(IoResult::None)
            }

            Err(e) => Err(e),
        }
    }

    fn tick(&mut self) -> Result<IoResult> {
        // Check for timeout-triggered results (e.g., escape key timeout, prefix timeout)
        let results = self.keybind_processor.tick();

        for result in results.into_iter().rev() {
            self.pending_results.push(result);
        }

        if let Some(result) = self.pending_results.pop()
            && let Some(read_result) = self.keybind_result_to_read_result(result)
        {
            return Ok(read_result);
        }

        Ok(IoResult::None)
    }

    fn write(&mut self, buf: &[u8]) -> Result<IoResult> {
        let filtered = self.apply_filter(buf);
        match std::io::stdout().write_all(&filtered) {
            Ok(()) => Ok(IoResult::Data(buf.to_vec())),
            Err(e) => Err(e),
        }
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
