use std::sync::Arc;

use anyhow::Result;

use database::{
    actions::cover_art::bake_cover_art_by_media_files,
    connection::{MainDbConnection, RecommendationDbConnection},
};

use crate::{query_cover_arts, Collection};

use tracing_subscriber::EnvFilter;
#[cfg(target_os = "android")]
use tracing_subscriber::fmt::format::Format;
#[cfg(target_os = "android")]
use tracing_logcat::{LogcatMakeWriter, LogcatTag};

pub async fn inject_cover_art_map(
    main_db: &MainDbConnection,
    recommend_db: Arc<RecommendationDbConnection>,
    collection: Collection,
) -> Result<Collection> {
    let files = query_cover_arts(main_db, recommend_db, collection.queries.clone()).await?;
    let cover_art_map = bake_cover_art_by_media_files(main_db, files).await?;

    Ok(Collection {
        id: collection.id,
        name: collection.name,
        queries: collection.queries,
        collection_type: collection.collection_type,
        cover_art_map,
        readonly: collection.readonly,
    })
}

#[cfg(not(target_os = "android"))]
pub fn init_logging() {
    let stdout_filter = EnvFilter::new(
        "symphonia_format_ogg=off,symphonia_core=off,symphonia_bundle_mp3::demuxer=off,sea_orm_migration::migrator=off,info",
    );

    tracing_subscriber::fmt()
        .with_env_filter(stdout_filter)
        .init();
}

#[cfg(target_os = "android")]
pub fn init_logging() {
    let tag = LogcatTag::Fixed(env!("CARGO_PKG_NAME").to_owned());
    let writer = LogcatMakeWriter::new(tag)
       .expect("Failed to initialize logcat writer");
    
    tracing_subscriber::fmt()
        .event_format(Format::default().with_level(false).without_time())
        .with_writer(writer)
        .with_ansi(false)
        .init();
}
