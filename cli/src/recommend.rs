use anyhow::{Context, Result, bail};
use database::entities::media_files;
use prettytable::{Table, format, row};
use serde_json::json;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use database::actions::file::{get_file_id_from_path, get_files_by_ids};
use database::actions::recommendation::get_recommendation_by_file_id;
use database::connection::{MainDbConnection, RecommendationDbConnection};

use crate::export::{
    OUTPUT_REQUIRED, UNSUPPORTED_FORMAT, check_and_correct_extension, save_as_m3u8,
};
use anyhow::Error;

pub struct RecommendMusicOptions<'a> {
    pub canonicalized_path: &'a Path,
    pub path: &'a Path,
    pub item_id: Option<i32>,
    pub file_path: Option<&'a PathBuf>,
    pub num: usize,
    pub format: Option<&'a str>,
    pub output: Option<&'a PathBuf>,
}

/// Finds the tracks most like the one named by `item_id` or `file_path`, and shows them in a
/// table or saves them in the `format` asked for.
///
/// Fails when neither names a track of the library, the recommendations or their tracks cannot
/// be read, the format is not one of `json` and `m3u8`, or the M3U8 playlist cannot be saved.
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
            .map_err(Error::msg)?
    } else {
        bail!("Either item_id or file_path must be provided.");
    };

    let recommendations: Vec<(u32, f32)> =
        get_recommendation_by_file_id(recommend_db, file_id, num)
            .context("Failed to get recommendations")?;

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
            let files_by_id: HashMap<_, _> = files.iter().map(|file| (file.id, file)).collect();
            let ranked = ids.iter().filter_map(|id| files_by_id.get(id).copied());
            let saved = save_as_m3u8(canonicalized_path, &canonicalized_path.join(output), ranked)?;
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
