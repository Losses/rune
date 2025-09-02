use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use log::error;
use prettytable::{Table, row};
use rust_decimal::prelude::ToPrimitive;

use database::actions::metadata::get_metadata_summary_by_file_ids;
use database::actions::mixes::query_mix_media_files;
use database::connection::{MainDbConnection, RecommendationDbConnection};
use database::entities::media_files;

use crate::recommend::check_and_correct_extension;

pub struct RecommendMixOptions<'a> {
    pub mix_parameters: &'a str,
    pub num: usize,
    pub format: Option<&'a str>,
    pub output: Option<&'a PathBuf>,
}

pub async fn mixes(
    main_db: &MainDbConnection,
    recommend_db: &RecommendationDbConnection,
    options: RecommendMixOptions<'_>,
) {
    let RecommendMixOptions {
        mix_parameters,
        num,
        format,
        output,
    } = options;

    // Convert mix_parameters to Vec<(String, String)>
    let mix_parameters_vec: Vec<(String, String)> = mix_parameters
        .split(';')
        .filter_map(|param| {
            // Trim leading and trailing whitespace
            let param = param.trim();
            // Find the position of the first '(' and the last ')'
            if let Some(start) = param.find('(')
                && let Some(end) = param.rfind(')')
            {
                // Extract the operator and parameter
                let operator = &param[..start];
                let parameter = &param[start + 1..end];
                // Trim leading and trailing whitespace and handle escape issues
                let operator = operator.trim().replace("\\(", "(").replace("\\)", ")");
                let parameter = parameter.trim().replace("\\(", "(").replace("\\)", ")");
                return Some((operator.to_string(), parameter.to_string()));
            }
            None
        })
        .collect();

    let files: Vec<media_files::Model> =
        match query_mix_media_files(main_db, recommend_db, mix_parameters_vec, 0, num).await {
            Ok(recommendations) => recommendations,
            Err(e) => {
                eprintln!("Failed to get recommendations: {e}");
                return;
            }
        };

    match format {
        Some("m3u8") => {
            save_mixes_as_m3u8(output, &files).await;
        }
        Some(_) => {
            eprintln!("Unsupported format. Supported formats are 'json' and 'm3u8'.");
        }
        _none => {
            display_mixes_in_table(main_db, &files).await;
        }
    }
}

pub async fn save_mixes_as_m3u8(output: Option<&PathBuf>, files: &Vec<media_files::Model>) {
    let output_path = match output {
        Some(path) => path,
        _none => {
            eprintln!("Output file path is required when format is specified");
            return;
        }
    };

    let corrected_path = check_and_correct_extension(output_path, "m3u8");
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
        let relative_path = Path::new(&file_info.directory).join(&file_info.file_name);
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

fn format_time(seconds: f64) -> String {
    let total_seconds = seconds.floor() as i32;

    let minutes = total_seconds / 60;
    let remaining_seconds = total_seconds % 60;

    let minutes_str = format!("{minutes:02}");
    let seconds_str = format!("{remaining_seconds:02}");

    format!("{minutes_str}:{seconds_str}")
}

pub async fn display_mixes_in_table(main_db: &MainDbConnection, files: &[media_files::Model]) {
    let file_ids = files.iter().map(|x| x.id).collect::<Vec<_>>();

    match get_metadata_summary_by_file_ids(main_db, file_ids).await {
        Ok(summaries) => {
            let mut table = Table::new();
            table.add_row(row![
                "ID",
                "Title",
                "Album",
                "Track Number",
                "Duration",
                "Cover Art ID"
            ]);

            for summary in summaries {
                table.add_row(row![
                    summary.id,
                    summary.title,
                    summary.album,
                    summary.track_number,
                    format_time(
                        summary
                            .duration
                            .to_f64()
                            .expect("Failed to convert duration")
                    ),
                    summary.cover_art_id.unwrap_or_default()
                ]);
            }

            table.printstd();
        }
        Err(e) => {
            error!("Failed to retrieve metadata summary: {e}");
        }
    }
}
