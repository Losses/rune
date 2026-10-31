//! CPU kernels shared by scanning and audio analysis.
//! Architecture selection stays behind this API; floating-point reductions retain
//! their original sample order. CRC parameters are part of the on-disk format.
pub mod audio;
pub mod checksum;
