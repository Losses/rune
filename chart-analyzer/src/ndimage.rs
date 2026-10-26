//! The `scipy.ndimage` 1-D filters the Python analyzer uses, with scipy's default boundary mode
//! `reflect`: the signal is mirrored about the edge of its first and last sample, so that
//! `d c b a | a b c d | d c b a` (the edge sample is repeated, unlike numpy's `reflect` pad).
//!
//! A window of `size` samples around output `i` covers `i - size / 2` to
//! `i + size - size / 2 - 1`, so an even size reaches one sample further back than forward, as
//! in scipy. The filters that run over every row of a spectrogram take a scratch buffer so that
//! they allocate nothing per row.

/// The index of sample `i` of an `n`-sample signal after reflecting (`i` may lie any distance
/// outside `0..n`).
pub fn reflect(i: isize, n: usize) -> usize {
    let period = 2 * n as isize;
    let i = i.rem_euclid(period);
    if i < n as isize {
        i as usize
    } else {
        (period - 1 - i) as usize
    }
}

/// `x` with `before` samples reflected on the left and `after` on the right, in `buf`.
fn pad_reflect<T: Copy>(x: &[T], before: usize, after: usize, buf: &mut Vec<T>) {
    buf.clear();
    let n = x.len();
    buf.extend((0..before + n + after).map(|k| x[reflect(k as isize - before as isize, n)]));
}

/// `scipy.ndimage.maximum_filter1d(x, size)` into `out` (same length as `x`). `buf` is scratch.
pub fn maximum_filter1d<T: Copy + PartialOrd>(
    x: &[T],
    size: usize,
    buf: &mut Vec<T>,
    out: &mut [T],
) {
    if x.is_empty() {
        return;
    }
    pad_reflect(x, size / 2, size - size / 2 - 1, buf);
    for (o, window) in out.iter_mut().zip(buf.windows(size)) {
        let mut m = window[0];
        for &v in &window[1..] {
            if v > m {
                m = v;
            }
        }
        *o = m;
    }
}

/// `scipy.ndimage.median_filter(x, size)` along one line into `out` (same length as `x`): the
/// middle value of the sorted window, which a sorted copy of it keeps up to date as the window
/// slides. `buf` and `sorted` are scratch.
pub fn median_filter1d(
    x: &[f32],
    size: usize,
    buf: &mut Vec<f32>,
    sorted: &mut Vec<f32>,
    out: &mut [f32],
) {
    if x.is_empty() {
        return;
    }
    pad_reflect(x, size / 2, size - size / 2 - 1, buf);
    sorted.clear();
    sorted.extend_from_slice(&buf[..size]);
    sorted.sort_by(f32::total_cmp);
    out[0] = sorted[size / 2];
    for i in 1..x.len() {
        let old = buf[i - 1];
        let at = sorted.partition_point(|v| v.total_cmp(&old).is_lt());
        sorted.remove(at);
        let new = buf[i + size - 1];
        let at = sorted.partition_point(|v| v.total_cmp(&new).is_lt());
        sorted.insert(at, new);
        out[i] = sorted[size / 2];
    }
}

/// `scipy.ndimage.uniform_filter1d(x, size)`: the mean of the window.
pub fn uniform_filter1d(x: &[f64], size: usize) -> Vec<f64> {
    let mut buf = Vec::new();
    pad_reflect(x, size / 2, size - size / 2 - 1, &mut buf);
    let mut sum: f64 = buf[..size.min(buf.len())].iter().sum();
    let mut out = Vec::with_capacity(x.len());
    for i in 0..x.len() {
        out.push(sum / size as f64);
        if i + size < buf.len() {
            sum += buf[i + size] - buf[i];
        }
    }
    out
}

/// scipy's `truncate` argument of `gaussian_filter1d`: the kernel reaches this many sigmas.
const GAUSSIAN_TRUNCATE: f64 = 4.0;

/// The kernel of `scipy.ndimage.gaussian_filter1d(x, sigma)`: weights for offsets `-r..=r`,
/// `r = int(4 sigma + 0.5)`, summing to one.
pub fn gaussian_kernel(sigma: f64) -> Vec<f64> {
    let radius = (GAUSSIAN_TRUNCATE * sigma + 0.5) as isize;
    let mut w: Vec<f64> = (-radius..=radius)
        .map(|x| (-0.5 / (sigma * sigma) * (x * x) as f64).exp())
        .collect();
    let total: f64 = w.iter().sum();
    w.iter_mut().for_each(|v| *v /= total);
    w
}

/// `scipy.ndimage.gaussian_filter1d(x, sigma)`.
pub fn gaussian_filter1d(x: &[f64], sigma: f64) -> Vec<f64> {
    let kernel = gaussian_kernel(sigma);
    let r = kernel.len() / 2;
    let mut buf = Vec::new();
    pad_reflect(x, r, r, &mut buf);
    buf.windows(kernel.len())
        .map(|window| window.iter().zip(&kernel).map(|(a, w)| a * w).sum())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(got: &[f64], want: &[f64]) {
        assert_eq!(got.len(), want.len());
        for (a, b) in got.iter().zip(want) {
            assert!((a - b).abs() < 1e-12, "{got:?} != {want:?}");
        }
    }

    #[test]
    fn reflect_repeats_the_edge_sample() {
        // d c b a | a b c d | d c b a for n = 4
        let got: Vec<usize> = (-4..8).map(|i| reflect(i, 4)).collect();
        assert_eq!(got, vec![3, 2, 1, 0, 0, 1, 2, 3, 3, 2, 1, 0]);
        // Further out than one period, and a signal shorter than the window.
        assert_eq!(reflect(-9, 4), 0);
        assert_eq!(reflect(-1, 1), 0);
    }

    const X: [f64; 9] = [3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0, 5.0];

    #[test]
    fn maximum_filter_matches_scipy() {
        let (mut buf, mut out) = (Vec::new(), [0.0; 9]);
        // maximum_filter1d(X, 3), and with size 4 (an even one reaches back further)
        maximum_filter1d(&X, 3, &mut buf, &mut out);
        close(&out, &[3.0, 4.0, 4.0, 5.0, 9.0, 9.0, 9.0, 6.0, 6.0]);
        maximum_filter1d(&X, 4, &mut buf, &mut out);
        close(&out, &[3.0, 4.0, 4.0, 5.0, 9.0, 9.0, 9.0, 9.0, 6.0]);
    }

    #[test]
    fn median_filter_matches_scipy() {
        let x: Vec<f32> = X.iter().map(|&v| v as f32).collect();
        let (mut buf, mut sorted, mut out) = (Vec::new(), Vec::new(), [0.0f32; 9]);
        // median_filter(X, size=5), reflect
        median_filter1d(&x, 5, &mut buf, &mut sorted, &mut out);
        assert_eq!(out, [3.0, 3.0, 3.0, 4.0, 4.0, 5.0, 5.0, 5.0, 5.0]);
        // median_filter(X, size=3)
        median_filter1d(&x, 3, &mut buf, &mut sorted, &mut out);
        assert_eq!(out, [3.0, 3.0, 1.0, 4.0, 5.0, 5.0, 6.0, 5.0, 5.0]);
        // A window longer than the signal reflects more than once: median_filter([1, 2, 3], 7)
        let mut short = [0.0f32; 3];
        median_filter1d(&[1.0, 2.0, 3.0], 7, &mut buf, &mut sorted, &mut short);
        assert_eq!(short, [2.0, 2.0, 2.0]);
    }

    #[test]
    fn uniform_filter_matches_scipy() {
        // uniform_filter1d(X, 3) and uniform_filter1d(X, 4)
        close(
            &uniform_filter1d(&X, 3),
            &[
                2.3333333333333335,
                2.6666666666666665,
                2.0,
                3.3333333333333335,
                5.0,
                5.333333333333333,
                5.666666666666667,
                4.333333333333333,
                5.333333333333333,
            ],
        );
        close(
            &uniform_filter1d(&X, 4),
            &[2.0, 2.75, 2.25, 2.75, 4.75, 4.25, 5.5, 5.5, 4.5],
        );
    }

    #[test]
    fn gaussian_filter_matches_scipy() {
        // gaussian_filter1d(X, 1.0): a radius of 4
        close(
            &gaussian_filter1d(&X, 1.0),
            &[
                2.457634119646063,
                2.3370390664753,
                2.565229619417447,
                3.139961538194237,
                4.770472244207859,
                5.702918974073985,
                4.995299373734586,
                4.892488689316841,
                5.138956374933682,
            ],
        );
        // gaussian_kernel(1.0) has 9 weights, summing to one
        let k = gaussian_kernel(1.0);
        assert_eq!(k.len(), 9);
        assert!((k.iter().sum::<f64>() - 1.0).abs() < 1e-15);
        // sigma 10.02 (0.1 * FPS) reaches int(4 * 10.02 + 0.5) = 40 frames
        assert_eq!(gaussian_kernel(10.0227272727).len(), 81);
    }
}
