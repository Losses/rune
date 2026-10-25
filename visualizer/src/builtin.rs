//! The scenes compiled into the program (`scenes/`, see `build.rs`), which every platform has
//! without any scene folder.

use crate::scene::Source;

/// The files of a scene: (path inside its folder, contents).
type Files = &'static [(&'static str, &'static [u8])];

include!(concat!(env!("OUT_DIR"), "/builtin_scenes.rs"));

/// A scene of the program.
#[derive(Debug, Clone)]
pub struct Builtin {
    /// The scene's `name`.
    pub name: String,
    pub source: Source,
}

/// Every built-in scene, by folder name. A scene whose manifest does not parse is left out (the
/// tests catch that).
pub fn scenes() -> Vec<Builtin> {
    SCENES
        .iter()
        .filter_map(|(_, files)| {
            let source = Source::Embedded(files);
            Some(Builtin {
                name: source.scene_name().ok()?,
                source,
            })
        })
        .collect()
}

/// The built-in scene called `name`.
pub fn find(name: &str) -> Option<Builtin> {
    scenes().into_iter().find(|scene| scene.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::DEFAULT_PRESET_NAME;

    #[test]
    fn every_scene_folder_is_a_scene_with_its_files() {
        let scenes = scenes();
        assert_eq!(scenes.len(), SCENES.len());
        let mut names: Vec<_> = scenes.iter().map(|s| s.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "Armillary",
                "Badge Tunnel",
                "CRT Wall",
                "Growth",
                "Neon Street",
                "Paperclip",
                "Torii Sea"
            ]
        );
        assert!(find(DEFAULT_PRESET_NAME).is_some());
        // The images of a scene came along.
        let (_, files) = SCENES.iter().find(|(dir, _)| *dir == "growth").unwrap();
        assert!(
            files
                .iter()
                .any(|(path, bytes)| *path == "assets/surface.png" && bytes.len() > 8)
        );
        // Hidden files stay out.
        assert!(
            SCENES
                .iter()
                .flat_map(|(_, files)| files.iter())
                .all(|(path, _)| !path.split('/').any(|part| part.starts_with('.')))
        );
    }
}
