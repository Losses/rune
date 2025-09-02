use prettytable::{Table, format, row};
use serde_json::json;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use database::actions::file::get_file_id_from_path;
use database::actions::file::get_files_by_ids;
use database::actions::recommendation::get_recommendation_by_file_id;
use database::connection::{MainDbConnection, RecommendationDbConnection};

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
) {
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
        match get_file_id_from_path(main_db, path, file_path).await {
            Ok(id) => id,
            Err(e) => {
                eprintln!("{e}");
                return;
            }
        }
    } else {
        eprintln!("Either item_id or file_path must be provided.");
        return;
    };

    let recommendations: Vec<(u32, f32)> =
        match get_recommendation_by_file_id(recommend_db, file_id, num) {
            Ok(recommendations) => recommendations,
            Err(e) => {
                eprintln!("Failed to get recommendations: {e}");
                return;
            }
        };

    // Get file details of recommendations
    let ids: Vec<i32> = recommendations.iter().map(|(id, _)| *id as i32).collect();
    let files = match get_files_by_ids(main_db, &ids).await {
        Ok(files) => files,
        Err(e) => {
            eprintln!("Failed to get files by IDs: {e}");
            return;
        }
    };

    match format {
        Some("json") => {
            save_recommendations_as_json(canonicalized_path, output, &recommendations).await;
        }
        Some("m3u8") => {
            save_recommendations_as_m3u8(canonicalized_path, output, path, &files).await;
        }
        Some(_) => {
            eprintln!("Unsupported format. Supported formats are 'json' and 'm3u8'.");
        }
        _none => {
            display_recommendations_in_table(path, &recommendations, &files);
        }
    }
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

pub async fn save_recommendations_as_m3u8(
    canonicalized_path: &Path,
    output: Option<&PathBuf>,
    path: &Path,
    files: &Vec<database::entities::media_files::Model>,
) {
    let output_path = match output {
        Some(path) => path,
        _none => {
            eprintln!("Output file path is required when format is specified");
            return;
        }
    };

    let corrected_path = check_and_correct_extension(&canonicalized_path.join(output_path), "m3u8");
    if corrected_path != *output_path {
        eprintln!("Warning: Output file extension corrected to .m3u8");
    }

    if let Some(parent) = corrected_path.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        eprintln!("Failed to create directories: {e}");
        return;
    }

    let mut file = match File::create(&corrected_path) {
        Ok(file) => file,
        Err(e) => {
            eprintln!("Failed to create file: {e}");
            return;
        }
    };

    if let Err(e) = file.write_all("#EXTM3U\n".as_bytes()) {
        eprintln!("Failed to write to file: {e}");
        return;
    }

    for file_info in files {
        let relative_path = path.join(&file_info.directory).join(&file_info.file_name);
        let relative_to_output =
            match pathdiff::diff_paths(&relative_path, corrected_path.parent().unwrap()) {
                Some(path) => path,
                None => {
                    eprintln!("Failed to calculate relative path");
                    return;
                }
            };

        if let Err(e) = writeln!(file, "{}", relative_to_output.display()) {
            eprintln!("Failed to write to file: {e}");
            return;
        }
    }

    println!(
        "Recommendations saved to M3U8 file: {}",
        corrected_path.to_str().unwrap()
    );
}

pub fn display_recommendations_in_table(
    path: &Path,
    recommendations: &Vec<(u32, f32)>,
    files: &[database::entities::media_files::Model],
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
