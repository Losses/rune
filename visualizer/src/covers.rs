//! The album covers a scene can read: one atlas of the track playing and random ones from the
//! library, built off the render thread and handed to the scene as a texture.

use std::path::PathBuf;

use image::imageops::{self, FilterType};
use image::{GenericImageView, RgbaImage};
use log::warn;

/// The atlas is `GRID` by `GRID` cells of `CELL` pixels.
pub const GRID: u32 = 8;
pub const CELL: u32 = 128;
pub const SIZE: u32 = GRID * CELL;
/// Cells of the atlas: the track playing, then the random covers.
pub const MAX_COVERS: usize = (GRID * GRID) as usize;
/// The side of the sharp copy of the cover playing, for the band of a cut-in, which shows a
/// close crop of it across the whole screen: 4 MiB, once.
pub const SHARP: u32 = 1024;

/// A finished atlas: RGBA8 (sRGB) pixels of `SIZE` by `SIZE`, row-major from the top left.
#[derive(Debug, PartialEq)]
pub struct Atlas {
    pub rgba: Vec<u8>,
    /// How many cells from the first are valid. Cell 0 is the track playing and counts even when
    /// it has no cover (it stays transparent); 0 only when there is no cover at all.
    pub count: u32,
}

/// One cell of the atlas on its own: the cover of the track playing, which changes without the
/// random ones changing with it.
#[derive(Debug, PartialEq)]
pub struct Cell {
    /// RGBA8 (sRGB) pixels of `CELL` by `CELL`; all clear if there is no cover.
    pub rgba: Vec<u8>,
    /// The same square, `SHARP` by `SHARP`, for the band of a cut-in, which a cell would blur;
    /// empty if there is no cover. It is made where the cell is, off the render thread.
    pub sharp: Vec<u8>,
    pub has_cover: bool,
}

impl Cell {
    /// The cell of `image`, centre-cropped and scaled (none: a clear cell).
    pub fn new(image: Option<&RgbaImage>) -> Self {
        match image {
            Some(image) => Self {
                rgba: fit_cell(image).into_raw(),
                sharp: fit_square(image, SHARP, FilterType::CatmullRom).into_raw(),
                has_cover: true,
            },
            None => Self {
                rgba: vec![0; (CELL * CELL * 4) as usize],
                sharp: Vec::new(),
                has_cover: false,
            },
        }
    }
}

/// How many cells of the scene's atlas are valid, as `frame.covers` tells it: the atlas's own
/// count, or 1 if there is no atlas yet but the track playing has a cover to show.
pub fn valid_cells(atlas: Option<&Atlas>, playing: Option<&Cell>) -> u32 {
    match atlas {
        Some(atlas) => atlas.count,
        None => u32::from(playing.is_some_and(|cell| cell.has_cover)),
    }
}

/// The largest centred square of `image`, scaled to a cell.
pub fn fit_cell(image: &RgbaImage) -> RgbaImage {
    fit_square(image, CELL, FilterType::Lanczos3)
}

/// The largest centred square of `image`, scaled to `side` by `side` with `filter`.
fn fit_square(image: &RgbaImage, side: u32, filter: FilterType) -> RgbaImage {
    let (w, h) = image.dimensions();
    let square = w.min(h);
    let square = image
        .view((w - square) / 2, (h - square) / 2, square, square)
        .to_image();
    imageops::resize(&square, side, side, filter)
}

/// The atlas of `current` (cell 0, if it has a cover) and then `random` in cells 1.., at most
/// `MAX_COVERS` cells. `None` when there is no cover at all.
pub fn pack(current: Option<&RgbaImage>, random: &[RgbaImage]) -> Option<Atlas> {
    if current.is_none() && random.is_empty() {
        return None;
    }
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    let cells = current
        .into_iter()
        .map(Some)
        .chain(std::iter::repeat_n(None, usize::from(current.is_none())))
        .chain(random.iter().map(Some))
        .take(MAX_COVERS);
    let mut count = 0;
    for (i, image) in cells.enumerate() {
        count += 1;
        let Some(image) = image else {
            continue;
        };
        let (x0, y0) = ((i as u32 % GRID) * CELL, (i as u32 / GRID) * CELL);
        let cell = fit_cell(image);
        let (rows, _) = cell.as_raw().as_chunks::<{ (CELL * 4) as usize }>();
        for (row, pixels) in rows.iter().enumerate() {
            let at = (((y0 + row as u32) * SIZE + x0) * 4) as usize;
            rgba[at..at + pixels.len()].copy_from_slice(pixels);
        }
    }
    Some(Atlas { rgba, count })
}

fn decode(path: &PathBuf) -> Option<RgbaImage> {
    let result = std::fs::read(path)
        .map_err(anyhow::Error::from)
        .and_then(|bytes| Ok(image::load_from_memory(&bytes)?));
    match result {
        Ok(image) => Some(image.to_rgba8()),
        Err(e) => {
            warn!("Skipping cover {}: {e:#}", path.display());
            None
        }
    }
}

/// Decode the cover of the track playing into its cell (a file that does not decode, or none, is
/// a clear cell).
pub fn build_cell(path: Option<&PathBuf>) -> Cell {
    Cell::new(path.and_then(decode).as_ref())
}

/// Decode the files and build the atlas; files that fail to decode are skipped.
pub fn build(current: Option<&PathBuf>, random: &[PathBuf]) -> Option<Atlas> {
    let current = current.and_then(decode);
    let random: Vec<RgbaImage> = random
        .iter()
        .take(MAX_COVERS - 1)
        .filter_map(decode)
        .collect();
    pack(current.as_ref(), &random)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, image::Rgba(rgba))
    }

    fn pixel(atlas: &Atlas, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * SIZE + x) * 4) as usize;
        atlas.rgba[at..at + 4].try_into().unwrap()
    }

    #[test]
    fn cells_are_row_major_from_the_top_left() {
        let current = solid(300, 300, [255, 0, 0, 255]);
        let random: Vec<RgbaImage> = (0..9).map(|i| solid(10, 10, [0, i * 20, 0, 255])).collect();
        let atlas = pack(Some(&current), &random).unwrap();
        assert_eq!(atlas.count, 10);
        assert_eq!(pixel(&atlas, 5, 5), [255, 0, 0, 255]);
        // Cell 1 is next to cell 0, cell 8 starts the second row, cell 10 is empty.
        assert_eq!(pixel(&atlas, CELL + 5, 5), [0, 0, 0, 255]);
        assert_eq!(pixel(&atlas, 5, CELL + 5), [0, 7 * 20, 0, 255]);
        assert_eq!(pixel(&atlas, CELL + 5, CELL + 5), [0, 8 * 20, 0, 255]);
        assert_eq!(pixel(&atlas, 2 * CELL + 5, CELL + 5), [0, 0, 0, 0]);
    }

    #[test]
    fn a_missing_current_cover_leaves_cell_zero_empty_but_counted() {
        let random = [solid(8, 8, [0, 0, 255, 255])];
        let atlas = pack(None, &random).unwrap();
        assert_eq!(atlas.count, 2);
        assert_eq!(pixel(&atlas, 5, 5), [0, 0, 0, 0]);
        assert_eq!(pixel(&atlas, CELL + 5, 5), [0, 0, 255, 255]);
    }

    #[test]
    fn a_cell_is_a_cropped_cover_or_clear() {
        let cell = Cell::new(Some(&solid(300, 100, [0, 255, 0, 255])));
        assert!(cell.has_cover);
        assert_eq!(cell.rgba.len(), (CELL * CELL * 4) as usize);
        assert_eq!(cell.rgba[..4], [0, 255, 0, 255]);
        // The sharp copy is the same square at its own size.
        assert_eq!(cell.sharp.len(), (SHARP * SHARP * 4) as usize);
        assert_eq!(cell.sharp[..4], [0, 255, 0, 255]);
        let clear = Cell::new(None);
        assert!(!clear.has_cover);
        assert!(clear.rgba.iter().all(|&b| b == 0));
        assert!(clear.sharp.is_empty());
        assert_eq!(build_cell(None), clear);
    }

    #[test]
    fn the_playing_cover_makes_one_valid_cell_before_any_atlas() {
        let cover = Cell::new(Some(&solid(4, 4, [1, 2, 3, 255])));
        let clear = Cell::new(None);
        assert_eq!(valid_cells(None, None), 0);
        assert_eq!(valid_cells(None, Some(&clear)), 0);
        assert_eq!(valid_cells(None, Some(&cover)), 1);
        let atlas = pack(None, &[solid(4, 4, [9, 9, 9, 255])]).unwrap();
        assert_eq!(valid_cells(Some(&atlas), Some(&clear)), 2);
        assert_eq!(valid_cells(Some(&atlas), None), 2);
    }

    #[test]
    fn no_covers_make_no_atlas() {
        assert_eq!(pack(None, &[]), None);
    }

    #[test]
    fn at_most_a_grid_of_cells() {
        let random: Vec<RgbaImage> = (0..100).map(|_| solid(4, 4, [9, 9, 9, 255])).collect();
        let atlas = pack(None, &random).unwrap();
        assert_eq!(atlas.count, MAX_COVERS as u32);
    }

    #[test]
    fn a_wide_cover_is_cropped_to_its_centre() {
        // Red left third, green middle, blue right third: only the middle square stays.
        let mut wide = solid(300, 100, [0, 255, 0, 255]);
        for y in 0..100 {
            for x in 0..100 {
                wide.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
                wide.put_pixel(x + 200, y, image::Rgba([0, 0, 255, 255]));
            }
        }
        let cell = fit_cell(&wide);
        assert_eq!(cell.dimensions(), (CELL, CELL));
        assert_eq!(cell.get_pixel(0, 64).0, [0, 255, 0, 255]);
        assert_eq!(cell.get_pixel(CELL - 1, 64).0, [0, 255, 0, 255]);
    }

    #[test]
    fn a_tall_cover_is_cropped_to_its_centre() {
        let mut tall = solid(100, 300, [0, 255, 0, 255]);
        for y in 0..100 {
            for x in 0..100 {
                tall.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            }
        }
        let cell = fit_cell(&tall);
        assert_eq!(cell.get_pixel(64, 0).0, [0, 255, 0, 255]);
    }
}
