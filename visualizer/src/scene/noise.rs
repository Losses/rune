//! A tileable blue-noise texture, made once: dithering and grain without visible patterns.

use std::sync::OnceLock;

pub const SIZE: usize = 64;
/// How far a placed point pushes its neighbours away (the energy's Gaussian).
const SIGMA: f32 = 1.9;
const RADIUS: i32 = 7;

/// Ranks 0..SIZE² laid out so that every threshold of them is evenly spread (the "void" half of
/// Ulichney's void-and-cluster, from an empty start): each next point goes where the energy of
/// the points placed so far is lowest.
fn ranks() -> Vec<u16> {
    let n = SIZE * SIZE;
    let mut energy = vec![0.0f32; n];
    // A tiny fixed jitter breaks the ties that would otherwise grow a regular grid.
    let mut seed = 0x9e37_79b9u32;
    for e in &mut energy {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        *e = (seed >> 8) as f32 * 1e-9;
    }
    let mut kernel = Vec::new();
    for dy in -RADIUS..=RADIUS {
        for dx in -RADIUS..=RADIUS {
            let d2 = (dx * dx + dy * dy) as f32;
            kernel.push((dx, dy, (-d2 / (2.0 * SIGMA * SIGMA)).exp()));
        }
    }
    let mut rank = vec![u16::MAX; n];
    for r in 0..n {
        let at = (0..n)
            .filter(|&i| rank[i] == u16::MAX)
            .min_by(|&a, &b| energy[a].total_cmp(&energy[b]))
            .expect("a free cell");
        rank[at] = r as u16;
        let (x, y) = ((at % SIZE) as i32, (at / SIZE) as i32);
        for &(dx, dy, w) in &kernel {
            let i = (y + dy).rem_euclid(SIZE as i32) as usize * SIZE
                + (x + dx).rem_euclid(SIZE as i32) as usize;
            energy[i] += w;
        }
    }
    rank
}

/// RGBA8: one blue-noise pattern in R and the same pattern shifted by half a tile in G, B, A,
/// so the channels are close to independent.
pub fn rgba() -> &'static [u8] {
    static TEXTURE: OnceLock<Vec<u8>> = OnceLock::new();
    TEXTURE.get_or_init(|| {
        let rank = ranks();
        let value = |x: usize, y: usize| {
            let r = rank[(y % SIZE) * SIZE + x % SIZE] as usize;
            ((r * 256) / (SIZE * SIZE)) as u8
        };
        let h = SIZE / 2;
        let mut out = Vec::with_capacity(SIZE * SIZE * 4);
        for y in 0..SIZE {
            for x in 0..SIZE {
                out.extend([
                    value(x, y),
                    value(x + h, y),
                    value(x, y + h),
                    value(x + h, y + h),
                ]);
            }
        }
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_level_appears_equally_and_neighbours_differ() {
        let t = rgba();
        let mut counts = [0u32; 256];
        for px in t.chunks(4) {
            counts[px[0] as usize] += 1;
        }
        assert!(counts.iter().all(|&c| c == 16), "a flat histogram");
        // Blue noise has little low-frequency energy: neighbours are far apart on average.
        let mut diff = 0u32;
        for y in 0..SIZE {
            for x in 0..SIZE - 1 {
                let a = t[(y * SIZE + x) * 4] as i32;
                let b = t[(y * SIZE + x + 1) * 4] as i32;
                diff += a.abs_diff(b);
            }
        }
        let mean = diff as f32 / (SIZE * (SIZE - 1)) as f32;
        // White noise would average 85.
        assert!(mean > 95.0, "mean neighbour difference {mean}");
    }
}
