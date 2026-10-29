//! The few numpy functions the Python analyzer leans on, with numpy's exact semantics where they
//! are not the obvious ones (the Hann window's formula, percentile's interpolation, `interp`'s
//! clamping, Python's banker's rounding). Each has a test against values numpy computed.

/// `np.hanning(n)`: the symmetric Hann window of `n` points (zero at both ends), as numpy writes
/// it, so that the values are bit for bit numpy's.
pub fn hanning(n: usize) -> Vec<f64> {
    if n == 1 {
        return vec![1.0];
    }
    let m = (n - 1) as f64;
    (0..n)
        .map(|i| {
            let k = (2 * i) as f64 - m;
            0.5 + 0.5 * (std::f64::consts::PI * k / m).cos()
        })
        .collect()
}

/// Python's `round(x)` for a float: halves go to the even neighbour, `round(0.5) == 0`.
pub fn round_half_even(x: f64) -> f64 {
    x.round_ties_even()
}

/// Python's `round(x, digits)` as the chart writes it: `x` rounded to the nearest decimal
/// number of `digits` places by its exact binary value (halves to the even one), which is what
/// formatting it with that many places does. Scaling it by a power of ten instead would round
/// the product, and make a tie of a number that lies just off one.
pub fn round_to(x: f64, digits: usize) -> f64 {
    if !x.is_finite() {
        return x;
    }
    format!("{x:.digits$}").parse().unwrap_or(x)
}

/// `np.percentile(x, q)` with the default linear interpolation: the value at the fractional rank
/// `(n - 1) * q / 100` of the sorted data. numpy's lerp is `a + (b - a) t` below one half and
/// `b - (b - a)(1 - t)` from there, which this keeps. 0 for empty input.
pub fn percentile(x: &[f64], q: f64) -> f64 {
    if x.is_empty() {
        return 0.0;
    }
    let mut sorted = x.to_vec();
    sorted.sort_by(f64::total_cmp);
    let virtual_index = (sorted.len() - 1) as f64 * (q / 100.0);
    let previous = virtual_index.floor();
    let t = virtual_index - previous;
    let a = sorted[previous as usize];
    let b = sorted[(previous as usize + 1).min(sorted.len() - 1)];
    let diff = b - a;
    if t >= 0.5 {
        b - diff * (1.0 - t)
    } else {
        a + diff * t
    }
}

/// `np.median(x)`: the middle value, the mean of the two middle ones for an even count. 0 for
/// empty input.
pub fn median(x: &[f64]) -> f64 {
    if x.is_empty() {
        return 0.0;
    }
    let mut sorted = x.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

/// `np.mean`, 0 for empty input.
pub fn mean(x: &[f64]) -> f64 {
    if x.is_empty() {
        0.0
    } else {
        x.iter().sum::<f64>() / x.len() as f64
    }
}

/// `np.std` (the population one, `ddof = 0`), 0 for empty input.
pub fn std(x: &[f64]) -> f64 {
    if x.is_empty() {
        return 0.0;
    }
    let m = mean(x);
    (x.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / x.len() as f64).sqrt()
}

/// `np.argmax`: the first of the equal maxima. 0 for empty input.
pub fn argmax(x: &[f64]) -> usize {
    let mut best = 0;
    for (i, &v) in x.iter().enumerate() {
        if v > x[best] {
            best = i;
        }
    }
    best
}

/// `np.argmin`: the first of the equal minima. 0 for empty input.
pub fn argmin(x: &[f64]) -> usize {
    let mut best = 0;
    for (i, &v) in x.iter().enumerate() {
        if v < x[best] {
            best = i;
        }
    }
    best
}

/// `np.interp(x, xp, fp)` for an increasing `xp`: linear between the points, the first and last
/// value of `fp` outside them. `fp` must be as long as `xp`, and not empty.
pub fn interp(x: f64, xp: &[f64], fp: &[f64]) -> f64 {
    let last = xp.len() - 1;
    if x < xp[0] {
        return fp[0];
    }
    if x >= xp[last] {
        return fp[last];
    }
    // The last point not after x.
    let j = xp.partition_point(|&p| p <= x) - 1;
    let slope = (fp[j + 1] - fp[j]) / (xp[j + 1] - xp[j]);
    slope * (x - xp[j]) + fp[j]
}

/// `np.searchsorted(a, v)` (side left): the first index whose value is not below `v`.
pub fn searchsorted(a: &[f64], v: f64) -> usize {
    a.partition_point(|&p| p < v)
}

/// `np.geomspace(start, stop, num)`: `num` points spaced evenly on a log scale, both ends exact.
pub fn geomspace(start: f64, stop: f64, num: usize) -> Vec<f64> {
    let (log_start, log_stop) = (start.log10(), stop.log10());
    let step = (log_stop - log_start) / (num - 1) as f64;
    let mut out: Vec<f64> = (0..num)
        .map(|i| 10f64.powf(i as f64 * step + log_start))
        .collect();
    out[0] = start;
    out[num - 1] = stop;
    out
}

/// numpy's pairwise summation of a contiguous run, which `np.sum` of a one-dimensional array (and
/// of a flat array of any shape) uses in the array's own type: fewer than 8 values are added in
/// order; up to 128 in eight interleaved partial sums, combined as a tree; more are split in
/// two halves (the first a multiple of 8) summed the same way. 0 for no values.
pub fn pairwise_sum<T>(a: &[T]) -> T
where
    T: Copy + Default + std::ops::Add<Output = T>,
{
    const BLOCK: usize = 128;
    let n = a.len();
    if n < 8 {
        return a.iter().fold(T::default(), |sum, &v| sum + v);
    }
    if n <= BLOCK {
        let mut r: [T; 8] = std::array::from_fn(|j| a[j]);
        let whole = n - n % 8;
        for chunk in a[8..whole].as_chunks::<8>().0.iter() {
            for (partial, &v) in r.iter_mut().zip(chunk) {
                *partial = *partial + v;
            }
        }
        let mut sum = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
        for &v in &a[whole..] {
            sum = sum + v;
        }
        return sum;
    }
    let half = n / 2 - (n / 2) % 8;
    pairwise_sum(&a[..half]) + pairwise_sum(&a[half..])
}

/// Python's `a % m` for a positive `m`, the result never negative.
pub fn modulo(a: i64, m: i64) -> i64 {
    a.rem_euclid(m)
}

/// Python's `a // m` for a positive `m`, rounding towards minus infinity.
pub fn floor_div(a: i64, m: i64) -> i64 {
    a.div_euclid(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-12, "{a} != {b}");
    }

    #[test]
    fn hanning_matches_numpy() {
        // np.hanning(8)
        let want = [
            0.0,
            0.18825509907063326,
            0.6112604669781572,
            0.9504844339512095,
            0.9504844339512095,
            0.6112604669781572,
            0.18825509907063326,
            0.0,
        ];
        for (a, b) in hanning(8).iter().zip(want) {
            assert!((a - b).abs() < 1e-15, "{a} != {b}");
        }
        assert_eq!(hanning(1), vec![1.0]);
        assert_eq!(hanning(5)[2], 1.0);
    }

    #[test]
    fn rounding_goes_to_even() {
        // Python: round(0.5), round(1.5), round(2.5), round(13.5), round(14.5), round(-0.5)
        let got: Vec<f64> = [0.5, 1.5, 2.5, 13.5, 14.5, -0.5]
            .map(round_half_even)
            .to_vec();
        assert_eq!(got, vec![0.0, 2.0, 2.0, 14.0, 14.0, -0.0]);
        // Python: round(0.126, 2), round(0.125, 2)
        close(round_to(0.126, 2), 0.13);
        close(round_to(0.125, 2), 0.12);
        // Exact ties go to the even digit, and a number just off a tie goes where it lies:
        // Python: round(0.375, 2), round(0.0625, 3), round(3.0245, 3),
        // round(3.7234999999999996, 3), round(-0.0004, 3)
        assert_eq!(round_to(0.375, 2), 0.38);
        assert_eq!(round_to(0.0625, 3), 0.062);
        assert_eq!(round_to(3.0245, 3), 3.025);
        assert_eq!(round_to(3.7234999999999996, 3), 3.723);
        assert_eq!(round_to(-0.0004, 3), 0.0);
    }

    #[test]
    fn percentile_matches_numpy() {
        let x = [3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0];
        // np.percentile(x, [98, 75, 5, 50, 0, 100])
        close(percentile(&x, 98.0), 8.58);
        close(percentile(&x, 75.0), 5.25);
        close(percentile(&x, 5.0), 1.0);
        close(percentile(&x, 50.0), 3.5);
        close(percentile(&x, 0.0), 1.0);
        close(percentile(&x, 100.0), 9.0);
        assert_eq!(percentile(&[], 50.0), 0.0);
    }

    #[test]
    fn median_and_std_match_numpy() {
        close(median(&[3.0, 1.0, 2.0]), 2.0);
        close(median(&[4.0, 1.0, 3.0, 2.0]), 2.5);
        close(std(&[1.0, 2.0, 3.0, 4.0]), 1.118033988749895);
    }

    #[test]
    fn argmax_takes_the_first_of_ties() {
        assert_eq!(argmax(&[1.0, 3.0, 3.0, 2.0]), 1);
        assert_eq!(argmin(&[2.0, 1.0, 1.0, 3.0]), 1);
    }

    #[test]
    fn interp_clamps_like_numpy() {
        let xp = [1.0, 2.0, 4.0];
        let fp = [10.0, 20.0, 0.0];
        // np.interp([0, 1, 1.5, 2, 3, 4, 5], xp, fp)
        let got: Vec<f64> = [0.0, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0]
            .iter()
            .map(|&x| interp(x, &xp, &fp))
            .collect();
        assert_eq!(got, vec![10.0, 10.0, 15.0, 20.0, 10.0, 0.0, 0.0]);
    }

    #[test]
    fn searchsorted_is_left() {
        let a = [0.0, 1.0, 1.0, 2.0];
        // np.searchsorted(a, [-1, 0, 1, 1.5, 2, 3])
        let got: Vec<usize> = [-1.0, 0.0, 1.0, 1.5, 2.0, 3.0]
            .map(|v| searchsorted(&a, v))
            .to_vec();
        assert_eq!(got, vec![0, 0, 1, 3, 3, 4]);
    }

    #[test]
    fn geomspace_matches_numpy() {
        // np.geomspace(60, 10000, 5) and np.geomspace(60, 10000, 25)[[1, 12, 23]]
        let got = geomspace(60.0, 10000.0, 5);
        let want = [
            60.0,
            215.58246717785062,
            774.5966692414836,
            2783.157683713741,
            10000.0,
        ];
        for (a, b) in got.iter().zip(want) {
            assert!((a - b).abs() < 1e-9 * b, "{a} != {b}");
        }
        let got = geomspace(60.0, 10000.0, 25);
        for (i, b) in [
            (1, 74.25544098781523),
            (12, 774.5966692414836),
            (23, 8080.215968260904),
        ] {
            assert!((got[i] - b).abs() < 1e-9 * b, "{} != {b}", got[i]);
        }
    }

    #[test]
    fn pairwise_sum_matches_numpy() {
        // x[i] = f32((i * 37) % 101) * 0.013 + 0.1 in float32 arithmetic; np.sum of it.
        let x = |n: usize| -> Vec<f32> {
            (0..n)
                .map(|i| ((i * 37 % 101) as f32) * 0.013f32 + 0.1f32)
                .collect()
        };
        for (n, want) in [
            (7, 4.236000061035156),
            (25, 18.125999450683594),
            (200, 150.11700439453125),
            (1000, 750.1300048828125),
            (5000, 3749.779052734375),
        ] {
            assert_eq!(f64::from(pairwise_sum(&x(n))), want, "{n}");
        }
        assert_eq!(pairwise_sum::<f64>(&[]), 0.0);
    }

    #[test]
    fn python_modulo() {
        assert_eq!(modulo(-1, 4), 3);
        assert_eq!(floor_div(-1, 4), -1);
        assert_eq!(floor_div(7, 4), 1);
    }
}
