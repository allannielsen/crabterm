pub mod charmap;
pub mod timestamp;

use std::collections::HashMap;

use crate::keybind::config::SettingValue;
pub use charmap::CharmapFilter;
pub use timestamp::TimestampFilter;

/// Trait for filters that transform data
pub trait IoFilter {
    /// Returns whether the filter is currently enabled
    fn enabled(&self) -> bool;

    /// Toggle the filter on/off
    fn toggle(&mut self);

    /// Filter output data (device -> terminal)
    fn filter_out(&mut self, buf: &[u8]) -> Vec<u8> {
        buf.to_vec()
    }

    /// Filter input data (terminal -> device)
    fn filter_in(&mut self, buf: &[u8]) -> Vec<u8> {
        buf.to_vec()
    }
}

/// Manages all available filters
pub struct FilterChain {
    timestamp_filter: TimestampFilter,
    charmap_filter: CharmapFilter,
}

impl FilterChain {
    pub fn new(settings: &HashMap<String, SettingValue>) -> Self {
        let mut timestamp_filter = TimestampFilter::new();
        timestamp_filter.configure(settings);

        let mut charmap_filter = CharmapFilter::new();
        charmap_filter.configure(settings);

        FilterChain {
            timestamp_filter,
            charmap_filter,
        }
    }

    /// Toggle a filter by name. Returns true if the filter exists.
    pub fn toggle(&mut self, name: &str) -> bool {
        match name {
            timestamp::NAME => {
                self.timestamp_filter.toggle();
                true
            }
            charmap::NAME => {
                self.charmap_filter.toggle();
                true
            }
            _ => false,
        }
    }

    /// Apply all active output filters (device -> terminal)
    pub fn filter_out(&mut self, buf: &[u8]) -> Vec<u8> {
        let mut output = buf.to_vec();

        if self.timestamp_filter.enabled() {
            output = self.timestamp_filter.filter_out(&output);
        }

        if self.charmap_filter.enabled() {
            output = self.charmap_filter.filter_out(&output);
        }

        output
    }

    /// Apply all active input filters (terminal -> device)
    pub fn filter_in(&mut self, buf: &[u8]) -> Vec<u8> {
        let mut output = buf.to_vec();

        if self.charmap_filter.enabled() {
            output = self.charmap_filter.filter_in(&output);
        }

        output
    }
}

impl Default for FilterChain {
    fn default() -> Self {
        Self::new(&HashMap::new())
    }
}
