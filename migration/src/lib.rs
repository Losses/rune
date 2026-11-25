use once_cell::sync::OnceCell;
use sea_orm_migration::sea_orm as sea_orm_migration_sea_orm;
use std::future;
pub mod catalog_identity;
pub use sea_orm_migration::prelude::*;

static NODE_ID: OnceCell<String> = OnceCell::new();

pub fn initialize_node_id(node_id: String) -> Result<(), String> {
    NODE_ID.set(node_id)
}

pub fn get_node_id() -> &'static str {
    NODE_ID.get().expect("Node ID has not been initialized")
}

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
mod m20231110_000019_create_playback_queue_table;
mod m20231117_000020_create_log_table;
mod m20250311_000021_create_genres_table;
mod m20250311_000022_create_media_file_genres_table;
mod m20250312_000023_create_media_file_fingerprint_table;
mod m20250312_000024_create_media_file_similarity_table;
mod m20250410_000025_add_hlc_columns;
mod m20250529_000026_create_sync_record_table;
mod m20251010_000027_add_index_cover_art_file_hash;
mod m20251010_000028_add_index_media_files_cover_art_id;

mod m20261031_000029_add_library_lookup_indexes;

mod m20261031_000030_add_analysis_version;

mod m20261101_000031_create_sync_store;
mod m20261101_000032_create_sync_library;
mod m20261101_000034_create_download_rules;
mod m20261111_000035_drop_playback_queue;
mod m20261111_000036_add_media_file_change_token;
mod m20261112_000037_rename_source_uuid;
mod m20261122_000038_add_analysis_source_identity;
mod m20261123_000039_settle_pre_release_tables;
mod m20261124_000040_create_sync_export_cursors;
mod retired;
#[cfg(feature = "test-fixtures")]
pub mod test_fixtures;

tokio::task_local! {
    static MIGRATION_ACTOR: Option<String>;
}

/// Run a migration invocation with the actor that owns legacy statistics.
pub async fn up_with_actor(
    db: &sea_orm_migration_sea_orm::DatabaseConnection,
    actor: Option<&str>,
    steps: Option<u32>,
) -> Result<(), sea_orm_migration_sea_orm::DbErr> {
    with_migration_actor(actor, Migrator::up(db, steps)).await
}

async fn with_migration_actor<F: future::Future>(actor: Option<&str>, future: F) -> F::Output {
    MIGRATION_ACTOR
        .scope(actor.map(str::to_owned), future)
        .await
}

fn migration_actor() -> Option<String> {
    MIGRATION_ACTOR.try_with(Clone::clone).ok().flatten()
}

/// How the migrations a database records compare with this version's.
#[derive(Debug, PartialEq, Eq)]
pub enum MigrationHistory {
    /// Every migration it records is one of this version's.
    Known,
    /// It records migrations newer than this version's: a newer Rune wrote it.
    Newer(Vec<String>),
    /// It records migrations this version does not know, none newer than its
    /// own. The migrator reports them.
    Unknown(Vec<String>),
}

impl MigrationHistory {
    /// The history of a database that records the migrations [`recorded`].
    pub fn of(recorded: &[String]) -> Self {
        let known: Vec<String> = Migrator::migrations()
            .iter()
            .map(|migration| migration.name().to_owned())
            .collect();
        let newest = known.iter().max();
        let unknown: Vec<String> = recorded
            .iter()
            .filter(|version| !known.contains(version))
            .cloned()
            .collect();
        if unknown.is_empty() {
            MigrationHistory::Known
        } else if unknown.iter().any(|version| Some(version) > newest) {
            MigrationHistory::Newer(unknown)
        } else {
            MigrationHistory::Unknown(unknown)
        }
    }
}

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20230701_000001_create_media_files_table::Migration),
            Box::new(m20230701_000002_create_media_metadata_table::Migration),
            Box::new(m20230701_000003_create_media_analysis_table::Migration),
            Box::new(retired::user_logs::Migration),
            Box::new(m20230701_000005_create_playlists_table::Migration),
            Box::new(retired::playlist_items::Migration),
            Box::new(m20230701_000006_create_media_file_playlists::Migration),
            Box::new(retired::smart_playlists::Migration),
            Box::new(retired::media_cover_arts::Migration),
            Box::new(m20230728_000008_create_media_cover_art_table::Migration),
            Box::new(m20230806_000009_create_artists_table::Migration),
            Box::new(m20230806_000010_create_media_file_artists_table::Migration),
            Box::new(m20230806_000011_create_albums_table::Migration),
            Box::new(m20230806_000012_create_media_file_albums_table::Migration),
            // Here rather than last: a database made before the first
            // release needs it before the migrations after 12.
            Box::new(m20261123_000039_settle_pre_release_tables::Migration),
            Box::new(m20230912_000013_create_mixes_table::Migration),
            Box::new(m20230912_000014_create_mix_queries_table::Migration),
            Box::new(m20230912_000015_create_media_file_stats_table::Migration),
            Box::new(m20230923_000016_seed_mixes::Migration),
            Box::new(m20231029_000017_create_search_index::Migration),
            Box::new(m20231107_000018_add_column_primary_color::Migration),
            Box::new(m20231110_000019_create_playback_queue_table::Migration),
            Box::new(m20231117_000020_create_log_table::Migration),
            Box::new(m20250311_000021_create_genres_table::Migration),
            Box::new(m20250311_000022_create_media_file_genres_table::Migration),
            Box::new(m20250312_000023_create_media_file_fingerprint_table::Migration),
            Box::new(m20250312_000024_create_media_file_similarity_table::Migration),
            Box::new(m20250410_000025_add_hlc_columns::Migration),
            Box::new(m20250529_000026_create_sync_record_table::Migration),
            Box::new(m20251010_000027_add_index_cover_art_file_hash::Migration),
            Box::new(m20251010_000028_add_index_media_files_cover_art_id::Migration),
            Box::new(m20261031_000029_add_library_lookup_indexes::Migration),
            Box::new(m20261031_000030_add_analysis_version::Migration),
            Box::new(m20261101_000031_create_sync_store::Migration),
            Box::new(m20261101_000032_create_sync_library::Migration),
            Box::new(m20261101_000034_create_download_rules::Migration),
            Box::new(m20261111_000035_drop_playback_queue::Migration),
            Box::new(m20261111_000036_add_media_file_change_token::Migration),
            Box::new(m20261112_000037_rename_source_uuid::Migration),
            Box::new(m20261122_000038_add_analysis_source_identity::Migration),
            Box::new(m20261124_000040_create_sync_export_cursors::Migration),
        ]
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    fn names(versions: &[&str]) -> Vec<String> {
        versions
            .iter()
            .map(|version| (*version).to_owned())
            .collect()
    }

    #[test]
    fn a_history_of_this_versions_migrations_is_known() {
        let all: Vec<String> = Migrator::migrations()
            .iter()
            .map(|migration| migration.name().to_owned())
            .collect();
        assert_eq!(MigrationHistory::of(&[]), MigrationHistory::Known);
        assert_eq!(MigrationHistory::of(&all[..3]), MigrationHistory::Known);
        assert_eq!(MigrationHistory::of(&all), MigrationHistory::Known);
    }

    #[test]
    fn the_migrations_of_builds_before_the_first_release_are_known() {
        // What a library made in August 2024 records.
        let recorded = names(&[
            "m20230701_000001_create_media_files_table",
            "m20230701_000004_create_user_logs_table",
            "m20230701_000006_create_playlist_items_table",
            "m20230701_000007_create_smart_playlists_table",
            "m20230728_000008_create_media_cover_arts_table",
            "m20230806_000012_create_media_file_albums_table",
        ]);
        assert_eq!(MigrationHistory::of(&recorded), MigrationHistory::Known);
        assert_eq!(
            MigrationHistory::of(&names(&["m20230701_000099_never_written"])),
            MigrationHistory::Unknown(names(&["m20230701_000099_never_written"]))
        );
    }

    #[test]
    fn a_migration_after_this_versions_newest_is_a_newer_history() {
        let recorded = names(&[
            "m20230701_000001_create_media_files_table",
            "m29991231_000999_from_a_later_version",
        ]);
        assert_eq!(
            MigrationHistory::of(&recorded),
            MigrationHistory::Newer(names(&["m29991231_000999_from_a_later_version"]))
        );
    }
}
