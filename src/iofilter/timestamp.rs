use std::collections::HashMap;
use std::io::Write;
use std::time::Instant;

use chrono::Local;

use super::IoFilter;
use crate::keybind::config::SettingValue;

pub const NAME: &str = "timestamp";
pub const SETTING_ABS: &str = "timestamp-abs";
pub const SETTING_REL: &str = "timestamp-rel";

pub struct TimestampFilter {
    enabled: bool,
    show_abs: bool,
    show_rel: bool,
    at_line_start: bool,
    last_output: Option<Instant>,
}

impl TimestampFilter {
    pub fn new() -> Self {
        TimestampFilter {
            enabled: false,
            show_abs: true,
            show_rel: false,
            at_line_start: true,
            last_output: None,
        }
    }

    pub fn configure(&mut self, settings: &HashMap<String, SettingValue>) {
        if let Some(value) = settings.get(SETTING_ABS).and_then(|v| v.as_bool()) {
            self.show_abs = value;
        }
        if let Some(value) = settings.get(SETTING_REL).and_then(|v| v.as_bool()) {
            self.show_rel = value;
        }
    }
}

impl Default for TimestampFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl IoFilter for TimestampFilter {
    fn enabled(&self) -> bool {
        self.enabled
    }

    fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }

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
                    if self.show_abs {
                        let now = Local::now();
                        write!(output, "{} ", now.format("%H:%M:%S%.3f")).unwrap();
                    }
                    if self.show_rel {
                        let elapsed = self
                            .last_output
                            .map(|t| t.elapsed())
                            .unwrap_or_default();
                        write!(output, "+{:>6.3} ", elapsed.as_secs_f64()).unwrap();
                    }
                    self.last_output = Some(Instant::now());
                    self.at_line_start = false;
                }
                output.push(byte);
            }
        }
        output
    }
}
