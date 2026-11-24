//! Small CPU-only benchmark; run with --release. Modes alternate each round.
use media_compute::audio::{AudioKernels, time_domain_stats};
use std::{hint::black_box, time::Instant};
fn scalar_stats(s: &[f32]) -> (f32, usize, f32) {
    let mut rms = 0.0;
    for &v in s {
        rms += v.powi(2);
    }
    rms = (rms / s.len() as f32).sqrt();
    let zcr = s
        .windows(2)
        .filter(|v| (v[0] >= 0.0 && v[1] < 0.0) || (v[0] < 0.0 && v[1] >= 0.0))
        .count();
    let mut energy = 0.0;
    for &v in s {
        energy += v.abs().powi(2);
    }
    (rms, zcr, energy)
}
fn main() {
    let kernels = AudioKernels::new();
    let input: Vec<_> = (0..1024).map(|i| (i as f32 * 0.19).sin()).collect();
    let window: Vec<_> = (0..1024).map(|i| (i as f32 * 0.03).cos()).collect();
    let mut out = vec![0.0; 1024];
    for round in 0..5 {
        for j in 0..4 {
            let mode = (j + round) % 4;
            let t = Instant::now();
            let mut it = 0u64;
            while t.elapsed().as_millis() < 250 {
                for _ in 0..64 {
                    let a = black_box(input.as_slice());
                    let b = black_box(window.as_slice());
                    let o = black_box(out.as_mut_slice());
                    match mode {
                        0 => {
                            // Give the scalar compiler the same slice bounds as the SIMD API.
                            assert_eq!(o.len(), a.len());
                            assert_eq!(o.len(), b.len());
                            for (i, sample) in o.iter_mut().enumerate() {
                                *sample = a[i] * b[i];
                            }
                            for i in 0..o.len() {
                                o[i] += a[i];
                            }
                            black_box(&o);
                        }
                        1 => {
                            kernels.apply_window(a, b, o);
                            kernels.add_assign(o, a);
                            black_box(&o);
                        }
                        2 => {
                            black_box(scalar_stats(a));
                        }
                        _ => {
                            black_box(time_domain_stats(a));
                        }
                    }
                    it += 1;
                }
            }
            println!(
                "round={round} mode={mode} ns_per_window={:.3}",
                t.elapsed().as_secs_f64() * 1e9 / it as f64
            );
        }
    }
}
