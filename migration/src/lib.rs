pub use sea_orm_migration::prelude::*;

mod m20230701_000001_create_media_files_table;
mod m20230701_000002_create_media_metadata_table;
mod m20230701_000003_create_media_analysis_table;
mod m20230701_000005_create_playlists_table;
mod m20230701_000006_create_media_file_playlists;
mod m20230728_000008_create_media_cover_art_table;
mod m20230806_000009_create_artists_table;
mod m20230806_000010_create_media_file_artists_table;
mod m20230806_000011_create_albums_table;
mod m20230806_000012_create_media_file_albums_table;
mod m20230912_000013_create_mixes_table;
mod m20230912_000014_create_mix_queries_table;
mod m20230912_000015_create_media_file_stats_table;
mod m20230923_000016_seed_mixes;
mod m20231029_000017_create_search_index;
mod m20231107_000018_add_column_primary_color;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20230701_000001_create_media_files_table::Migration),
            Box::new(m20230701_000002_create_media_metadata_table::Migration),
            Box::new(m20230701_000003_create_media_analysis_table::Migration),
            Box::new(m20230701_000005_create_playlists_table::Migration),
            Box::new(m20230701_000006_create_media_file_playlists::Migration),
            Box::new(m20230728_000008_create_media_cover_art_table::Migration),
            Box::new(m20230806_000009_create_artists_table::Migration),
            Box::new(m20230806_000010_create_media_file_artists_table::Migration),
            Box::new(m20230806_000011_create_albums_table::Migration),
            Box::new(m20230806_000012_create_media_file_albums_table::Migration),
            Box::new(m20230912_000013_create_mixes_table::Migration),
            Box::new(m20230912_000014_create_mix_queries_table::Migration),
            Box::new(m20230912_000015_create_media_file_stats_table::Migration),
            Box::new(m20230923_000016_seed_mixes::Migration),
            Box::new(m20231029_000017_create_search_index::Migration),
            Box::new(m20231107_000018_add_column_primary_color::Migration),
        ]
    }
}
