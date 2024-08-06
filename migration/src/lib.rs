pub use sea_orm_migration::prelude::*;

mod m20230701_000001_create_media_files_table;
mod m20230701_000002_create_media_metadata_table;
mod m20230701_000003_create_media_analysis_table;
mod m20230701_000004_create_user_logs_table;
mod m20230701_000005_create_playlists_table;
mod m20230701_000006_create_playlist_items_table;
mod m20230701_000007_create_smart_playlists_table;
mod m20230728_000008_create_media_cover_art_table;
mod m20230806_000009_create_artists_table;
mod m20230806_000010_create_media_file_artists_table;
mod m20230806_000011_create_albums_table;
mod m20230806_000012_create_media_file_albums_table;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20230701_000001_create_media_files_table::Migration),
            Box::new(m20230701_000002_create_media_metadata_table::Migration),
            Box::new(m20230701_000003_create_media_analysis_table::Migration),
            Box::new(m20230701_000004_create_user_logs_table::Migration),
            Box::new(m20230701_000005_create_playlists_table::Migration),
            Box::new(m20230701_000006_create_playlist_items_table::Migration),
            Box::new(m20230701_000007_create_smart_playlists_table::Migration),
            Box::new(m20230728_000008_create_media_cover_art_table::Migration),
            Box::new(m20230806_000009_create_artists_table::Migration),
            Box::new(m20230806_000010_create_media_file_artists_table::Migration),
            Box::new(m20230806_000011_create_albums_table::Migration),
            Box::new(m20230806_000012_create_media_file_albums_table::Migration),
        ]
    }
}
