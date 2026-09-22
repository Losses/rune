#[cfg(target_arch = "x86_64")]
use super::simple_avx::*;
#[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
use super::simple_neon::*;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
use super::simple_sse::*;
use crate::unaligned_vector::{BinaryQuantized, UnalignedVector};

#[cfg(target_arch = "x86_64")]
const MIN_DIM_SIZE_AVX: usize = 32;

#[cfg(any(
    target_arch = "x86",
    target_arch = "x86_64",
    all(target_arch = "aarch64", target_feature = "neon")
))]
const MIN_DIM_SIZE_SIMD: usize = 16;

pub fn euclidean_distance(u: &UnalignedVector<f32>, v: &UnalignedVector<f32>) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx")
            && is_x86_feature_detected!("fma")
            && u.len() >= MIN_DIM_SIZE_AVX
        {
            return unsafe { euclid_similarity_avx(u, v) };
        }
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("sse") && u.len() >= MIN_DIM_SIZE_SIMD {
            return unsafe { euclid_similarity_sse(u, v) };
        }
    }

    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    {
        if std::arch::is_aarch64_feature_detected!("neon") && u.len() >= MIN_DIM_SIZE_SIMD {
            return unsafe { euclid_similarity_neon(u, v) };
        }
    }

    euclidean_distance_non_optimized(u, v)
}

// Don't use dot-product: avoid catastrophic cancellation in
// https://github.com/spotify/annoy/issues/314.
pub fn euclidean_distance_non_optimized(u: &UnalignedVector<f32>, v: &UnalignedVector<f32>) -> f32 {
    u.iter().zip(v.iter()).map(|(u, v)| (u - v) * (u - v)).sum()
}

pub fn dot_product(u: &UnalignedVector<f32>, v: &UnalignedVector<f32>) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx")
            && is_x86_feature_detected!("fma")
            && u.len() >= MIN_DIM_SIZE_AVX
        {
            return unsafe { dot_similarity_avx(u, v) };
        }
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("sse") && u.len() >= MIN_DIM_SIZE_SIMD {
            return unsafe { dot_similarity_sse(u, v) };
        }
    }

    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    {
        if std::arch::is_aarch64_feature_detected!("neon") && u.len() >= MIN_DIM_SIZE_SIMD {
            return unsafe { dot_similarity_neon(u, v) };
        }
    }

    dot_product_non_optimized(u, v)
}

pub fn dot_product_non_optimized(u: &UnalignedVector<f32>, v: &UnalignedVector<f32>) -> f32 {
    u.iter().zip(v.iter()).map(|(a, b)| a * b).sum()
}

/// For the binary quantized dot product:
/// 1. We need to multiply two scalars, in our case the only allowed values are -1 and 1:
/// ```text
/// -1 * -1 =  1
/// -1 *  1 = -1
///  1 * -1 = -1
///  1 *  1 =  1
/// ```
///
/// This looks like a negative xor already, if we replace the -1 by the binary quantized 0, and the 1 stays 1s:
/// ```text
/// ! 0 ^ 0 = 1
/// ! 0 ^ 1 = 0
/// ! 1 ^ 0 = 0
/// ! 1 ^ 1 = 1
/// ```
/// Is equivalent to `!(a ^ b)`.
///
/// 2. Then we need to do the sum of the results:
///    2.1 First we must do the sum of the operation on the `Word`s
///        /!\ We must be careful here because `1 - 0` actually translates to `1 - 1 = 0`.
///        `word.count_ones() - word.count_zeroes()` should do it:
/// ```text
///  00 => -2
///  01 => 0
///  10 => 0
///  11 => 2
/// ```
///  /!\ We must also take care to use signed integer to be able to go into negatives
///
/// 2.2 Finally we must sum the result of all the words
///     - By taking care of not overflowing: The biggest vectors contains like 5000 dimensions, a i16 could be enough. A i32 should be perfect.
///     - We can do the sum straight away without any more tricks
///     - We can cast the result to an f32 as expected
pub fn dot_product_binary_quantized(
    u: &UnalignedVector<BinaryQuantized>,
    v: &UnalignedVector<BinaryQuantized>,
) -> f32 {
    u.as_bytes()
        .iter()
        .zip(v.as_bytes())
        .map(|(u, v)| {
            let ret = !(u ^ v);
            ret.count_ones() as i32 - ret.count_zeros() as i32
        })
        .sum::<i32>() as f32
}
