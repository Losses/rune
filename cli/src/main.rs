use std::{fs, path::PathBuf, sync::Arc};

use clap::{Parser, Subcommand};
use directories::ProjectDirs;
use dunce::canonicalize;
use log::{error, info};
use prettytable::{Table, row};
use tracing_subscriber::filter::EnvFilter;

use database::{
    actions::{
        cover_art::scan_cover_arts,
        metadata::{empty_progress_callback, get_metadata_summary_by_file_ids, scan_audio_library},
        search::search_for,
    },
    connection::{connect_main_db, connect_recommendation_db},
};
use fsio::FsIo;

use rune::{
    analysis::*,
    index::index_audio_library,
    mix::{RecommendMixOptions, mixes},
    playback::*,
    recommend::*,
};
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "Media Manager")]
#[command(about = "A CLI tool for managing media libraries", long_about = None)]
struct Cli {
    /// The root path of the media library
    #[arg()]
    library: Option<PathBuf>,

    /// The subcommand to run
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan the audio library
    Scan,

    /// Index the audio files in the library
    Index,

    /// Analyze the audio files in the library
    Analyze {
        /// The compute device to use (cpu/gpu)
        #[arg(short, long, default_value = "gpu")]
        computing_device: String,
    },

    /// Show information of the track in the library
    Info {
        /// A list of file IDs to retrieve information for
        #[arg(short, long, num_args = 1..)]
        file_ids: Vec<i32>,
    },

    /// Play audio files in the library
    Play {
        /// The mode to play audio files
        #[arg()]
        mode: Option<String>,

        /// The ID of the file to play (used with playById mode)
        #[arg(short, long)]
        id: Option<i32>,
    },

    /// Recommend music
    Recommend {
        /// The ID of the item to get recommendations for
        #[arg(short, long, group = "recommend_group")]
        item_id: Option<i32>,

        /// The file path of the music to get recommendations for
        #[arg(short = 'p', long, group = "recommend_group")]
        file_path: Option<PathBuf>,

        /// The number of recommendations to retrieve
        #[arg(short, long, default_value_t = 10)]
        num: usize,

        /// The format of the output (json or m3u8)
        #[arg(short, long)]
        format: Option<String>,

        /// The output file path (required if format is specified)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Recommend mixes
    Mix {
        /// The mix parameters to get recommendations for
        #[arg(short, long)]
        mix_parameters: String,

        /// The number of recommendations to retrieve
        #[arg(short, long, default_value_t = 10)]
        num: usize,

        /// The format of the output (json or m3u8)
        #[arg(short, long)]
        format: Option<String>,

        /// The output file path (required if format is specified)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Search the audio library
    Search {
        /// The search query string
        #[arg(short, long)]
        query: String,

        /// The number of results to retrieve per collection type
        #[arg(short, long, default_value_t = 10)]
        num: usize,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let filter = EnvFilter::new(
        "symphonia_format_ogg=off,symphonia_core=off,symphonia_bundle_mp3::demuxer=off,tantivy::directory=off,tantivy::indexer=off,sea_orm_migration::migrator=off,info",
    );

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_test_writer()
        .init();
    // Determine the path from either the option or the positional argument
    let path = cli.library.expect("Path is required");

    let proj_dirs = ProjectDirs::from("ci", "not", "rune").unwrap();

    let config_dir = proj_dirs.config_dir();
    let config_path = config_dir.to_path_buf();

    if config_path.exists() {
        if config_path.is_file() {
            panic!("Config directory path is a file: {config_path:?}");
        }
    } else {
        fs::create_dir_all(&config_path).unwrap();
    }

    let canonicalized_path = match canonicalize(&path) {
        Ok(path) => path,
        Err(e) => {
            error!("Failed to canonicalize path: {e}");
            return;
        }
    };

    let lib_path = match canonicalized_path.to_str() {
        Some(path) => path,
        _ => {
            error!("Invalid path, could not convert to string");
            return;
        }
    };
    let fsio = Arc::new(FsIo::new());

    // TODO: INTEGRATING THE CLIENT ID LATER
    let main_db = match connect_main_db(&fsio, lib_path, None, "").await {
        Ok(db) => db,
        Err(e) => {
            error!("Failed to connect to main database: {e}");
            return;
        }
    };

    let analysis_db = match connect_recommendation_db(&fsio, lib_path, None).await {
        Ok(db) => db,
        Err(e) => {
            error!("Failed to connect to analysis database: {e}");
            return;
        }
    };

    let nid_path = config_path.join("nid");

    fsio.ensure_file(&nid_path).await.unwrap();
    info!("Checking nid file at: {nid_path:?}");

    let content = fsio.read_to_string(&nid_path);
    let node_id = match content {
        Ok(content) => {
            let trimmed = content.trim();
            match Uuid::parse_str(trimmed) {
                Ok(uuid) => {
                    info!("Found valid UUID: {uuid}");
                    uuid
                }
                Err(_) => {
                    info!("Invalid UUID in nid file, generating new one");
                    let new_uuid = Uuid::new_v4();
                    fsio.write_string(&nid_path, &new_uuid.to_string())
                        .await
                        .unwrap();
                    new_uuid
                }
            }
        }
        Err(_) => {
            info!("nid file not found, creating new one");
            let new_uuid = Uuid::new_v4();
            fsio.write_string(&nid_path, &new_uuid.to_string())
                .await
                .unwrap();
            new_uuid
        }
    }.to_string();

    match &cli.command {
        Commands::Scan => {
            let _ = scan_audio_library(
                &fsio,
                &main_db,
                &node_id,
                &path,
                true,
                false,
                empty_progress_callback,
                None,
            )
            .await;
            let _ = scan_cover_arts(fsio, &main_db, &path, "", 10, |_now, _total| {}, None).await;
            info!("Library scanned successfully.");
        }
        Commands::Index => {
            index_audio_library(&main_db).await;
        }
        Commands::Analyze { computing_device } => {
            analyze_audio_library(
                computing_device.as_str().into(),
                fsio,
                &main_db,
                &analysis_db,
                &path,
                "",
            )
            .await;
        }
        Commands::Info { file_ids } => {
            match get_metadata_summary_by_file_ids(&main_db, file_ids.to_vec()).await {
                Ok(summaries) => {
                    let mut table = Table::new();
                    table.add_row(row![
                        "ID",
                        "Artist",
                        "Album",
                        "Title",
                        "Track Number",
                        "Duration",
                        "Cover Art ID"
                    ]);

                    for summary in summaries {
                        table.add_row(row![
                            summary.id,
                            summary.artist,
                            summary.album,
                            summary.title,
                            summary.track_number,
                            summary.duration,
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
        // In the main function, update the match statement for Commands::Play
        Commands::Play { mode, id } => match mode.as_deref() {
            Some("random") => {
                play_random(&main_db, &canonicalized_path).await;
            }
            Some("id") => {
                if let Some(file_id) = id {
                    play_by_id(&main_db, &canonicalized_path, *file_id).await;
                } else {
                    error!("File ID is required for playById mode.");
                }
            }
            _ => {
                info!("Mode not implemented!");
            }
        },
        Commands::Recommend {
            item_id,
            file_path,
            num,
            format,
            output,
        } => {
            recommend_music(
                &main_db,
                &analysis_db,
                RecommendMusicOptions {
                    canonicalized_path: &canonicalized_path,
                    path: &path,
                    item_id: *item_id,
                    file_path: file_path.as_ref(),
                    num: *num,
                    format: format.as_ref().map(|x| x.as_str()),
                    output: output.as_ref(),
                },
            )
            .await;
        }
        Commands::Mix {
            mix_parameters,
            num,
            format,
            output,
        } => {
            mixes(
                &main_db,
                &analysis_db,
                RecommendMixOptions {
                    mix_parameters,
                    num: *num,
                    format: format.as_ref().map(|x| x.as_str()),
                    output: output.as_ref(),
                },
            )
            .await;
        }
        Commands::Search { query, num } => match search_for(&main_db, query, None, *num).await {
            Ok(results) => {
                for (collection_type, ids) in results {
                    info!("{collection_type:?}: {ids:?}");
                }
            }
            Err(e) => {
                error!("Search failed: {e}");
            }
        },
    }
}
