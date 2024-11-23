pub mod analysis;
pub mod shared_utils;
mod tests;
mod utils;
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
