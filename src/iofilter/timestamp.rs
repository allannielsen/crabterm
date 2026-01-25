use std::io::Write;

use chrono::Local;

use super::IoFilter;

pub struct TimestampFilter {
    at_line_start: bool,
}

impl TimestampFilter {
    pub fn new() -> Self {
        TimestampFilter {
            at_line_start: true,
        }
    }
}

impl Default for TimestampFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl IoFilter for TimestampFilter {
    fn filter_out(&mut self, buf: &[u8]) -> Vec<u8> {
        let mut output = Vec::new();
        for &byte in buf {
            if byte == b'\n' {
                output.push(byte);
                self.at_line_start = true;
            } else if byte == b'\r' {
                output.push(byte);
            } else {
                if self.at_line_start {
                    let now = Local::now();
                    write!(output, "{} ", now.format("%H:%M:%S%.3f")).unwrap();
                    self.at_line_start = false;
                }
                output.push(byte);
            }
        }
        output
    }
}
