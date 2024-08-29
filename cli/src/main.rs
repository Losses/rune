use clap::{Parser, Subcommand};
use dunce::canonicalize;
use log::error;
use rune::index::index_audio_library;
use std::path::PathBuf;
use tracing_subscriber::filter::EnvFilter;

use database::actions::metadata::scan_audio_library;
use database::connection::{connect_main_db, connect_recommendation_db, connect_search_db};

use rune::analysis::*;
use rune::playback::*;
use rune::recommend::*;

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
    Analyze,

    /// Play audio files in the library
    Play {
        /// Currently, we only support play audio files randomly
        #[arg()]
        mode: Option<String>,
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
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let filter = EnvFilter::new(
        "symphonia_format_ogg=off,symphonia_core=off,sea_orm_migration::migrator=off,info",
    );

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_test_writer()
        .init();

    // Determine the path from either the option or the positional argument
    let path = cli.library.expect("Path is required");

    let canonicalized_path = match canonicalize(&path) {
        Ok(path) => path,
        Err(e) => {
            error!("Failed to canonicalize path: {}", e);
            return;
        }
    };

    let lib_path = match canonicalized_path.to_str() {
        Some(path) => path,
        None => {
            error!("Invalid path, could not convert to string");
            return;
        }
    };

    let main_db = match connect_main_db(lib_path).await {
        Ok(db) => db,
        Err(e) => {
            error!("Failed to connect to main database: {}", e);
            return;
        }
    };

    let analysis_db = match connect_recommendation_db(lib_path) {
        Ok(db) => db,
        Err(e) => {
            error!("Failed to connect to analysis database: {}", e);
            return;
        }
    };

    let mut search_db = match connect_search_db(lib_path) {
        Ok(db) => db,
        Err(e) => {
            error!("Failed to connect to analysis database: {}", e);
            return;
        }
    };

    match &cli.command {
        Commands::Scan => {
            scan_audio_library(&main_db, &mut search_db, &path, true).await;
            println!("Library scanned successfully.");
        }
        Commands::Index => {
            index_audio_library(&main_db, &mut search_db).await;
        }
        Commands::Analyze => {
            analyze_audio_library(&main_db, &analysis_db, &path).await;
        }
        Commands::Play { mode } => {
            if mode.as_deref() == Some("random") {
                play_random(&main_db, &canonicalized_path).await;
            } else {
                println!("Mode not implemented!");
            }
        }
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
    }
}
