use anyhow::{Context, Result, bail};
use database::entities::media_files;
use dunce::canonicalize;
use prettytable::{Table, format, row};
use serde_json::json;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::Write;
use std::path::{self, Path, PathBuf};
use uuid::Uuid;

use database::actions::file::{get_file_id_from_path, get_files_by_ids};
use database::actions::recommendation::get_recommendation_by_file_id;
use database::connection::{MainDbConnection, RecommendationDbConnection};

pub const OUTPUT_REQUIRED: &str = "Output file path is required when format is specified";
pub const UNSUPPORTED_FORMAT: &str = "Unsupported format. Supported formats are 'json' and 'm3u8'.";

pub struct RecommendMusicOptions<'a> {
    pub canonicalized_path: &'a Path,
    pub path: &'a Path,
    pub item_id: Option<i32>,
    pub file_path: Option<&'a PathBuf>,
    pub num: usize,
    pub format: Option<&'a str>,
    pub output: Option<&'a PathBuf>,
}

pub async fn recommend_music(
    main_db: &MainDbConnection,
    recommend_db: &RecommendationDbConnection,
    options: RecommendMusicOptions<'_>,
) -> Result<()> {
    let RecommendMusicOptions {
        canonicalized_path,
        path,
        item_id,
        file_path,
        num,
        format,
        output,
    } = options;
    let file_id = if let Some(item_id) = item_id {
        item_id
    } else if let Some(file_path) = file_path {
        get_file_id_from_path(main_db, path, file_path)
            .await
            .map_err(anyhow::Error::msg)?
    } else {
        bail!("Either item_id or file_path must be provided.");
    };

    let recommendations: Vec<(u32, f32)> =
        get_recommendation_by_file_id(recommend_db, file_id, num)
            .context("Failed to get recommendations")?;

    // Get file details of recommendations
    let ids: Vec<i32> = recommendations.iter().map(|(id, _)| *id as i32).collect();
    let files = get_files_by_ids(main_db, &ids)
        .await
        .context("Failed to get files by IDs")?;

    match format {
        Some("json") => {
            save_recommendations_as_json(canonicalized_path, output, &recommendations).await;
        }
        Some("m3u8") => {
            let output = output.context(OUTPUT_REQUIRED)?;
            let saved = save_as_m3u8(canonicalized_path, &canonicalized_path.join(output), &files)?;
            println!("Recommendations saved to M3U8 file: {}", saved.display());
        }
        Some(_) => bail!(UNSUPPORTED_FORMAT),
        _none => {
            display_recommendations_in_table(path, &recommendations, &files);
        }
    }
    Ok(())
}

pub async fn save_recommendations_as_json(
    canonicalized_path: &Path,
    output: Option<&PathBuf>,
    recommendations: &Vec<(u32, f32)>,
) {
    let output_path = match output {
        Some(path) => path,
        _none => {
            eprintln!("Output file path is required when format is specified");
            return;
        }
    };

    let corrected_path = check_and_correct_extension(&canonicalized_path.join(output_path), "json");
    if corrected_path != *output_path {
        eprintln!("Warning: Output file extension corrected to .json");
    }

    if let Some(parent) = corrected_path.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        eprintln!("Failed to create directories: {e}");
        return;
    }

    let json_data = json!(recommendations);
    let mut file = match File::create(&corrected_path) {
        Ok(file) => file,
        Err(e) => {
            eprintln!("Failed to create file: {e}");
            return;
        }
    };

    if let Err(e) = file.write_all(json_data.to_string().as_bytes()) {
        eprintln!("Failed to write to file: {e}");
        return;
    }

    println!("Recommendations saved to JSON file.");
}

/// Saves `files` as an M3U8 playlist at `output`, each track as the path to it from the
/// playlist's folder. `library` is the canonical root that the tracks' directories are relative
/// to; a relative `output` is taken from the working directory.
///
/// The playlist takes the place of `output` only when it is complete, so an export that fails
/// leaves a file that was already there as it was. Returns where the playlist was saved.
pub fn save_as_m3u8(
    library: &Path,
    output: &Path,
    files: &[media_files::Model],
) -> Result<PathBuf> {
    let corrected = check_and_correct_extension(output, "m3u8");
    if corrected != output {
        eprintln!("Warning: Output file extension corrected to .m3u8");
    }
    let name = corrected
        .file_name()
        .with_context(|| format!("{} names no file", corrected.display()))?;
    let folder = path::absolute(&corrected)
        .with_context(|| format!("Failed to resolve {}", corrected.display()))?
        .parent()
        .map(Path::to_path_buf)
        .with_context(|| format!("{} has no folder", corrected.display()))?;
    fs::create_dir_all(&folder)
        .with_context(|| format!("Failed to create {}", folder.display()))?;
    // The library root is canonical, and a path between two folders is only right when both are.
    let folder =
        canonicalize(&folder).with_context(|| format!("Failed to resolve {}", folder.display()))?;

    let mut playlist = String::from("#EXTM3U\n");
    for file in files {
        let track = library.join(&file.directory).join(&file.file_name);
        let from_playlist = pathdiff::diff_paths(&track, &folder).with_context(|| {
            format!(
                "No path leads from {} to {}",
                folder.display(),
                track.display()
            )
        })?;
        writeln!(playlist, "{}", from_playlist.display())?;
    }

    let saved = folder.join(name);
    let temp = folder.join(format!(
        ".{}-{}.tmp",
        name.to_string_lossy(),
        Uuid::new_v4()
    ));
    let result = (|| -> Result<()> {
        fs::write(&temp, playlist)
            .with_context(|| format!("Failed to write {}", temp.display()))?;
        fs::rename(&temp, &saved)
            .with_context(|| format!("Failed to replace {}", saved.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result?;
    Ok(saved)
}

pub fn display_recommendations_in_table(
    path: &Path,
    recommendations: &Vec<(u32, f32)>,
    files: &[media_files::Model],
) {
    let mut table = Table::new();
    table.add_row(row!["ID", "Distance", "File Path"]);
    table.set_format(*format::consts::FORMAT_NO_LINESEP_WITH_TITLE);

    for (id, distance) in recommendations {
        let file_info = files.iter().find(|f| f.id == *id as i32);
        if let Some(file_info) = file_info {
            let file_path = path.join(&file_info.directory).join(&file_info.file_name);
            table.add_row(row![
                format!("{:0>5}", id),
                format!("{:.4}", distance),
                file_path.display()
            ]);
        }
    }

    table.printstd();
}

pub fn check_and_correct_extension(path: &Path, expected_extension: &str) -> PathBuf {
    if path.extension().and_then(|ext| ext.to_str()) != Some(expected_extension) {
        let mut corrected_path = path.to_path_buf();
        corrected_path.set_extension(expected_extension);
        corrected_path
    } else {
        path.to_path_buf()
    }
}
