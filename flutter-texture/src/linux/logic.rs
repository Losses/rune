//! The bookkeeping of the Linux textures that needs no OpenGL: which imported images to keep, how
//! to upload rows, what is left to delete. Plain data, so it is tested on every host.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

/// The memory of a Vulkan image imported into OpenGL, and the texture on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Imported {
    pub image_id: u64,
    pub image_set: u64,
    /// The memory object.
    pub memory: u32,
    pub texture: u32,
}

/// Take out the images that are no longer needed: those of another set than `set` (the producer
/// no longer draws into them; with `None`, no set is drawn into), apart from those in `keep` (the
/// engine may still sample them). Returns them for deleting.
pub(crate) fn prune(
    images: &mut Vec<Imported>,
    set: Option<u64>,
    keep: [Option<u64>; 2],
) -> Vec<Imported> {
    let (kept, dead) = images
        .drain(..)
        .partition(|image| Some(image.image_set) == set || keep.contains(&Some(image.image_id)));
    *images = kept;
    dead
}

/// OpenGL objects that only a populate can delete, since Flutter's context is current only there.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Garbage {
    pub textures: Vec<u32>,
    pub memory_objects: Vec<u32>,
    pub semaphores: Vec<u32>,
}

impl Garbage {
    pub(crate) fn add_images(&mut self, images: impl IntoIterator<Item = Imported>) {
        for image in images {
            self.textures.push(image.texture);
            self.memory_objects.push(image.memory);
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.textures.is_empty() && self.memory_objects.is_empty() && self.semaphores.is_empty()
    }
}

/// How rows of pixels go into a texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rows {
    /// Tightly packed: in one go.
    Tight,
    /// Padded to this many pixels: in one go with `GL_UNPACK_ROW_LENGTH` (OpenGL ES 3, or
    /// `GL_EXT_unpack_subimage`), else row by row.
    RowLength(i32),
    /// Padded by bytes that are no whole pixel: row by row.
    EachRow,
}

/// How rows of `width` RGBA pixels, `stride` bytes apart, go into a texture.
pub(crate) fn rows(width: u32, stride: usize) -> Rows {
    let row = width as usize * 4;
    if stride == row {
        Rows::Tight
    } else if stride.is_multiple_of(4)
        && let Ok(length) = i32::try_from(stride / 4)
    {
        Rows::RowLength(length)
    } else {
        Rows::EachRow
    }
}

/// Whether `name` is one of the space-separated `extensions` (`glGetString(GL_EXTENSIONS)`).
pub(crate) fn has_extension(extensions: &str, name: &str) -> bool {
    extensions
        .split_ascii_whitespace()
        .any(|extension| extension == name)
}

/// Log the first few occurrences of an event, then every 300th.
pub(crate) fn should_log(count: u64) -> bool {
    count <= 3 || count.is_multiple_of(300)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(image_id: u64, image_set: u64) -> Imported {
        Imported {
            image_id,
            image_set,
            memory: image_id as u32 * 10,
            texture: image_id as u32 * 10 + 1,
        }
    }

    #[test]
    fn prune_keeps_the_set_drawn_into_and_what_the_engine_may_sample() {
        let mut images = vec![image(1, 1), image(2, 1), image(3, 2), image(4, 2)];
        // A frame of set 2 in image 3, after one in image 2 of the old set.
        let dead = prune(&mut images, Some(2), [Some(3), Some(2)]);
        assert_eq!(dead, [image(1, 1)]);
        assert_eq!(images, [image(2, 1), image(3, 2), image(4, 2)]);

        // The next frame of set 2: the old set is gone.
        let dead = prune(&mut images, Some(2), [Some(4), Some(3)]);
        assert_eq!(dead, [image(2, 1)]);
        assert_eq!(images, [image(3, 2), image(4, 2)]);
    }

    #[test]
    fn prune_after_falling_back_to_pixels_keeps_only_the_last_image() {
        let mut images = vec![image(1, 1), image(2, 1)];
        // The first pixel frame replaced a frame in image 2.
        assert_eq!(prune(&mut images, None, [Some(2), None]), [image(1, 1)]);
        // The next one replaced pixels.
        assert_eq!(prune(&mut images, None, [None, None]), [image(2, 1)]);
        assert!(images.is_empty());
    }

    #[test]
    fn garbage_takes_textures_and_their_memory() {
        let mut garbage = Garbage::default();
        assert!(garbage.is_empty());
        garbage.add_images([image(1, 1), image(2, 1)]);
        garbage.semaphores.push(7);
        assert_eq!(
            garbage,
            Garbage {
                textures: vec![11, 21],
                memory_objects: vec![10, 20],
                semaphores: vec![7],
            }
        );
        assert!(!garbage.is_empty());
    }

    #[test]
    fn rows_go_in_one_go_unless_padded_by_parts_of_pixels() {
        assert_eq!(rows(64, 256), Rows::Tight);
        assert_eq!(rows(60, 256), Rows::RowLength(64));
        assert_eq!(rows(1, 6), Rows::EachRow);
    }

    #[test]
    fn extensions_match_whole_names() {
        let extensions = "GL_EXT_memory_object GL_EXT_memory_object_fd GL_OES_rgb8_rgba8";
        assert!(has_extension(extensions, "GL_EXT_memory_object"));
        assert!(has_extension(extensions, "GL_EXT_memory_object_fd"));
        assert!(!has_extension(extensions, "GL_EXT_semaphore"));
        assert!(!has_extension(extensions, "GL_EXT_memory"));
        assert!(!has_extension("", "GL_EXT_memory_object"));
    }

    #[test]
    fn logging_thins_out() {
        let logged: Vec<u64> = (1..=900).filter(|&n| should_log(n)).collect();
        assert_eq!(logged, [1, 2, 3, 300, 600, 900]);
    }
}
