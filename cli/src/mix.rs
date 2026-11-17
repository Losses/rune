use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use log::error;
use prettytable::{Table, row};
use rust_decimal::prelude::ToPrimitive;

use database::actions::metadata::get_metadata_summary_by_file_ids;
use database::actions::mixes::query_mix_media_files;
use database::connection::{MainDbConnection, RecommendationDbConnection};
use database::entities::media_files;

use crate::export::{OUTPUT_REQUIRED, UNSUPPORTED_FORMAT, save_as_m3u8};

pub struct RecommendMixOptions<'a> {
    /// The canonical root of the library.
    pub library: &'a Path,
    pub mix_parameters: &'a str,
    pub num: usize,
    pub format: Option<&'a str>,
    pub output: Option<&'a PathBuf>,
}

/// Runs the mix query in `mix_parameters` and shows its tracks in a table, or saves them as an
/// M3U8 playlist.
///
/// Fails when the query cannot be run, the format is not `m3u8`, or the playlist cannot be
/// saved.
pub async fn mixes(
    main_db: &MainDbConnection,
    recommend_db: &RecommendationDbConnection,
    options: RecommendMixOptions<'_>,
) -> Result<()> {
    let RecommendMixOptions {
        library,
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
        query_mix_media_files(main_db, recommend_db, mix_parameters_vec, 0, num, &[])
            .await
            .context("Failed to get recommendations")?;

    match format {
        Some("m3u8") => {
            let saved = save_as_m3u8(library, output.context(OUTPUT_REQUIRED)?, &files)?;
            println!("Recommendations saved to M3U8 file: {}", saved.display());
        }
        Some(_) => bail!(UNSUPPORTED_FORMAT),
        _none => {
            display_mixes_in_table(main_db, &files).await;
        }
    }
    Ok(())
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
                    summary
                        .track_number
                        .map_or_else(String::new, |number| number.to_string()),
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
