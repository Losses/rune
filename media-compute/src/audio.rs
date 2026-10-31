use fearless_simd::{Level, dispatch, prelude::*};

/// Select once per analyzer, not once per sample. No SIMD types escape this API.
pub struct AudioKernels {
    level: Level,
}

impl Default for AudioKernels {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioKernels {
    pub fn new() -> Self {
        Self {
            level: Level::new(),
        }
    }

    pub fn apply_window(&self, input: &[f32], window: &[f32], output: &mut [f32]) {
        assert_eq!(window.len(), output.len());
        assert!(input.len() >= output.len());
        dispatch!(self.level, simd => window_kernel(simd, input, window, output));
    }

    /// Element-wise accumulation, preserving the order of additions across frames.
    pub fn add_assign(&self, output: &mut [f32], input: &[f32]) {
        assert_eq!(input.len(), output.len());
        dispatch!(self.level, simd => add_kernel(simd, output, input));
    }
}

#[inline(always)]
fn window_kernel<S: Simd>(simd: S, input: &[f32], window: &[f32], output: &mut [f32]) {
    let n = S::f32s::LEN;
    let end = output.len() / n * n;
    for i in (0..end).step_by(n) {
        let a = S::f32s::from_slice(simd, &input[i..i + n]);
        let b = S::f32s::from_slice(simd, &window[i..i + n]);
        (a * b).store_slice(&mut output[i..i + n]);
    }
    for i in end..output.len() {
        output[i] = input[i] * window[i];
    }
}

#[inline(always)]
fn add_kernel<S: Simd>(simd: S, output: &mut [f32], input: &[f32]) {
    let n = S::f32s::LEN;
    let end = output.len() / n * n;
    for i in (0..end).step_by(n) {
        let a = S::f32s::from_slice(simd, &output[i..i + n]);
        let b = S::f32s::from_slice(simd, &input[i..i + n]);
        (a + b).store_slice(&mut output[i..i + n]);
    }
    for i in end..output.len() {
        output[i] += input[i];
    }
}

/// Share the square and traversal for RMS and energy, without reassociating the
/// sum or introducing FMA. Zero crossings retain the original comparison rules.
pub fn time_domain_stats(signal: &[f32]) -> (f32, usize, f32) {
    if signal.is_empty() {
        return (0.0, 0, 0.0);
    }
    let mut sum = 0.0f32;
    let mut crossings = 0;
    let mut previous = signal[0];
    for &value in signal {
        sum += value * value;
        if (previous >= 0.0 && value < 0.0) || (previous < 0.0 && value >= 0.0) {
            crossings += 1;
        }
        previous = value;
    }
    ((sum / signal.len() as f32).sqrt(), crossings, sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatched_kernels_preserve_scalar_bits_and_tails() {
        let special = [
            0.0f32,
            -0.0,
            f32::MIN_POSITIVE,
            f32::from_bits(1),
            -1.0,
            0.25,
            f32::MAX,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ];
        for level in [Level::baseline(), Level::new()] {
            let kernels = AudioKernels { level };
            for offset in 0..16 {
                for len in 0..130 {
                    let input: Vec<_> = (0..len + offset)
                        .map(|i| special[i % special.len()])
                        .collect();
                    let input = &input[offset..];
                    let window: Vec<_> = (0..len).map(|i| (i as f32 + 1.0) / 133.0).collect();
                    let expected: Vec<_> = input.iter().zip(&window).map(|(a, b)| a * b).collect();
                    let mut out = vec![0.0; len];
                    kernels.apply_window(input, &window, &mut out);
                    assert_eq!(
                        out.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                        expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
                    );
                    let expected: Vec<_> = out.iter().zip(&window).map(|(a, b)| a + b).collect();
                    kernels.add_assign(&mut out, &window);
                    assert_eq!(
                        out.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                        expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
                    );
                }
            }
        }
    }

    #[test]
    fn fused_statistics_preserve_sequential_reductions() {
        let mut state = 42u32;
        let signal: Vec<_> = (0..8192)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state as i32) as f32 / i32::MAX as f32
            })
            .collect();
        for len in (0..128).chain([1024, 4096, 8192]) {
            let data = &signal[..len];
            let mut rms = 0.0f32;
            for &v in data {
                rms += v.powi(2);
            }
            let mut energy = 0.0f32;
            for &v in data {
                energy += v.abs().powi(2);
            }
            let zcr = data
                .windows(2)
                .filter(|v| (v[0] >= 0.0 && v[1] < 0.0) || (v[0] < 0.0 && v[1] >= 0.0))
                .count();
            if len > 0 {
                rms = (rms / len as f32).sqrt();
            }
            let actual = time_domain_stats(data);
            assert_eq!(actual.0.to_bits(), rms.to_bits());
            assert_eq!(actual.1, zcr);
            assert_eq!(actual.2.to_bits(), energy.to_bits());
        }
        assert_eq!(time_domain_stats(&[-1.0, -0.0, 0.0, -1.0, 1.0]).1, 3);
    }
}
