pub mod timestamp;

use std::collections::HashMap;

pub use timestamp::TimestampFilter;

/// Trait for filters that transform output data
pub trait IoFilter {
    /// Returns whether the filter is currently enabled
    fn enabled(&self) -> bool;

    /// Toggle the filter on/off
    fn toggle(&mut self);

    /// Filter the input bytes and return the filtered output
    fn filter_out(&mut self, buf: &[u8]) -> Vec<u8> {
        buf.to_vec()
    }
}

/// Manages all available filters
pub struct FilterChain {
    timestamp_filter: TimestampFilter,
}

impl FilterChain {
    pub fn new(settings: &HashMap<String, bool>) -> Self {
        let mut timestamp_filter = TimestampFilter::new();
        timestamp_filter.configure(settings);

        FilterChain { timestamp_filter }
    }

    /// Toggle a filter by name. Returns true if the filter exists.
    pub fn toggle(&mut self, name: &str) -> bool {
        match name {
            timestamp::NAME => {
                self.timestamp_filter.toggle();
                true
            }
            _ => false,
        }
    }

    /// Apply all active filters to the output
    pub fn filter_out(&mut self, buf: &[u8]) -> Vec<u8> {
        let mut output = buf.to_vec();

        if self.timestamp_filter.enabled() {
            output = self.timestamp_filter.filter_out(&output);
        }

        output
    }
}

impl Default for FilterChain {
    fn default() -> Self {
        Self::new(&HashMap::new())
    }
}
