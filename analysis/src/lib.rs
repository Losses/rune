pub mod analysis;
pub mod shared_utils;
mod wgpu_fft;
mod utils;
mod tests;

cfg_if::cfg_if! {
    if #[cfg(feature = "bench")] {
        pub mod legacy;
        pub mod analyzer;
    } else {
        mod legacy;
        mod analyzer;
    }
}