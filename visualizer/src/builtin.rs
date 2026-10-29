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

/// What a built-in scene's assets owe their makers: the scene's name and the `source` of each
/// asset that has one, as written (they carry the licence and any change made).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credit {
    pub scene: String,
    pub assets: Vec<String>,
}

/// The credits of every built-in scene that has any, by folder name.
pub fn credits() -> Vec<Credit> {
    scenes()
        .into_iter()
        .filter_map(|scene| {
            let assets = scene.source.asset_sources().ok()?;
            (!assets.is_empty()).then_some(Credit {
                scene: scene.name,
                assets,
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
                "CRT Wall",
                "Emoji Tunnel",
                "Growth",
                "Neon Street",
                "Paperclip",
                "Torii Sea"
            ]
        );
        assert!(find(DEFAULT_PRESET_NAME).is_some());
        assert_eq!(find("Emoji Tunnel").unwrap().name, "Emoji Tunnel");
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

    #[test]
    fn credits_come_back_under_their_scene_and_name_twemoji() {
        let credits = credits();
        assert!(!credits.is_empty());
        let names: Vec<_> = scenes().into_iter().map(|s| s.name).collect();
        for credit in &credits {
            assert!(names.contains(&credit.scene), "{}", credit.scene);
            assert!(!credit.assets.is_empty());
            assert!(credit.assets.iter().all(|line| !line.trim().is_empty()));
        }
        // Twemoji's licence asks for credit where its graphics are shown.
        let twemoji = credits
            .iter()
            .find(|credit| credit.assets.iter().any(|line| line.contains("Twemoji")))
            .expect("a scene credits Twemoji");
        let line = twemoji
            .assets
            .iter()
            .find(|l| l.contains("Twemoji"))
            .unwrap();
        assert!(line.contains("CC-BY 4.0"));
        // A scene with no outside assets has no entry.
        assert!(credits.len() < scenes().len());
    }

    /// The raw fields of `struct Music` that a built-in scene must not read, each with what it
    /// reads instead (docs/music.md, "Raw fields"): the runtime works out their fallbacks once,
    /// so that every scene hears the music the same way.
    const REPLACED: &[(&str, &str)] = &[
        ("bpm", "tempo, locked"),
        ("feel", "tempo"),
        ("song_feel", "tempo"),
        ("beat_confidence", "locked"),
        ("meter_confidence", "locked"),
        ("pulse", "locked"),
        ("section_confidence", "cues"),
        ("beat", "clock"),
        ("beats_per_bar", "meter"),
        ("energy", "drive, intensity"),
        ("loudness", "drive, intensity"),
        ("section_energy", "drive, cue"),
        ("section_left", "cue_in"),
        ("sections", "cues"),
        (
            "has_chart",
            "nothing: a scene does not ask whether there is a chart",
        ),
        ("harmonic", "notes"),
        ("lead", "notes"),
        ("lead_onset", "notes"),
        ("bass", "band_low"),
        ("mids", "band_mid"),
        ("treble", "band_high"),
    ];

    /// The fields read as `music.<name>` in WGSL source.
    fn music_fields(source: &str) -> Vec<(usize, &str)> {
        let mut found = Vec::new();
        for (line, text) in source.lines().enumerate() {
            let code = text.split("//").next().unwrap_or_default();
            for (at, _) in code.match_indices("music.") {
                let before = code[..at].chars().next_back();
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let rest = &code[at + "music.".len()..];
                let end = rest
                    .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .unwrap_or(rest.len());
                found.push((line + 1, &rest[..end]));
            }
        }
        found
    }

    #[test]
    fn music_fields_are_found_by_whole_name_outside_comments() {
        let source = "let a = music.beats + music.beat;\n// music.bpm in a comment\nlet b = remusic.feel + music.tempo;";
        assert_eq!(
            music_fields(source),
            [(1, "beats"), (1, "beat"), (3, "tempo")]
        );
    }

    #[test]
    fn built_in_scenes_read_the_signals_rather_than_the_raw_fields_they_replace() {
        let mut misses = Vec::new();
        for (dir, files) in SCENES {
            for (path, bytes) in files.iter().filter(|(path, _)| path.ends_with(".wgsl")) {
                let source = std::str::from_utf8(bytes).unwrap();
                for (line, field) in music_fields(source) {
                    if let Some((_, instead)) = REPLACED.iter().find(|(name, _)| *name == field) {
                        misses.push(format!(
                            "{dir}/{path}:{line}: music.{field} (read {instead})"
                        ));
                    }
                }
            }
        }
        assert!(misses.is_empty(), "{}", misses.join("\n"));
    }

    /// A built-in scene takes what it wants of the music through `[[input]]` and `[[bind]]`
    /// (see "Bindings" in `docs/music.md`), which fall back where the music cannot be measured
    /// and which a lab can edit; a read of `music.` in its WGSL does neither.
    #[test]
    fn no_built_in_scene_reads_music_directly() {
        let mut reads = Vec::new();
        for (dir, files) in SCENES {
            for (path, bytes) in files.iter().filter(|(path, _)| path.ends_with(".wgsl")) {
                let source = std::str::from_utf8(bytes).unwrap();
                for (line, field) in music_fields(source) {
                    reads.push(format!("{dir}/{path}:{line}: music.{field}"));
                }
            }
        }
        assert!(
            reads.is_empty(),
            "bind these to inputs instead:\n{}",
            reads.join("\n")
        );
    }
}
