pub mod timestamp;

pub use timestamp::TimestampFilter;

/// Trait for filters that transform output data
pub trait IoFilter {
    /// Filter the input bytes and return the filtered output
    fn filter_out(&mut self, buf: &[u8]) -> Vec<u8> {
        buf.to_vec()
    }
}
