/// Persisted feature schema. Increment whenever analysis semantics change.
pub const CURRENT_ANALYSIS_VERSION: i32 = 2;
pub const ANALYSIS_SAMPLE_RATE: u32 = 11025;

pub mod analysis;
mod tests;
pub mod utils;
mod wgpu_fft;

cfg_if::cfg_if! {
    if #[cfg(feature = "bench")] {
        pub mod analyzer;
    } else {
        mod analyzer;
    }
}
