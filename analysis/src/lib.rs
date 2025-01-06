pub mod analysis;
mod tests;
pub mod utils;
mod wgpu_fft;

cfg_if::cfg_if! {
    if #[cfg(feature = "bench")] {
        pub mod legacy;
        pub mod analyzer;
    } else {
        mod legacy;
        mod analyzer;
    }
}
