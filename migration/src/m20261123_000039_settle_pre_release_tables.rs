//! Bring the tables of a database made before the first release (July to
//! September 2024) to the ones migrations 1 to 12 make today, keeping every
//! row, so the migrations after them run on it as on any other.
//!
//! Those builds changed migrations 1 to 12 in place and recorded some under
//! names since retired ([`super::retired`]), so such a database can differ
//! from today's tables in these ways, all handled here:
//!
//! - `playlist_items` (before 2024-08-22, its file column `file_id`) is
//!   today's `media_file_playlists`, and `media_cover_arts` (before
//!   2024-07-27) today's `media_cover_art`. Today's migrations made empty
//!   tables of the new names beside them in this run; the old tables take
//!   their places, which keeps the references to them.
//! - `user_logs` and `smart_playlists` were never written to and go.
//! - Columns added later in place: `media_files.cover_art_id`,
//!   `sample_rate` and `duration` (zero until the next scan reads the file
//!   again, which it does for every file a migration brought in, as none
//!   has a change token), the analysis features, `media_file_albums.
//!   track_number`, and `group` for playlists, artists and albums.
//! - `media_analysis` once kept `sample_rate` and `duration` itself.
//! - A file could be linked to the same artist twice; one link is kept.
//!
//! A database has these tables only if it was made before the first
//! release, so on any other this does nothing. It runs right after
//! migration 12 for such a database; on one that already applied later
//! migrations it runs last and finds nothing.

use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::ConnectionTrait;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Tables only a pre-release database has.
const PRE_RELEASE_TABLES: [&str; 4] = [
    "user_logs",
    "smart_playlists",
    "playlist_items",
    "media_cover_arts",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }

    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut pre_release = false;
        for table in PRE_RELEASE_TABLES {
            pre_release |= manager.has_table(table).await?;
        }
        if !pre_release {
            return Ok(());
        }
        take_renamed_tables(manager).await?;
        drop_unwritten_tables(manager).await?;
        settle_media_files(manager).await?;
        settle_analysis(manager).await?;
        settle_groups(manager).await?;
        drop_repeated_links(manager).await?;
        if !manager
            .has_column("media_file_albums", "track_number")
            .await?
        {
            add_column(
                manager,
                "media_file_albums",
                ColumnDef::new(Alias::new("track_number"))
                    .integer()
                    .null()
                    .to_owned(),
            )
            .await?;
        }
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // The pre-release shape is not made again.
        Ok(())
    }
}

async fn is_empty(manager: &SchemaManager<'_>, table: &str) -> Result<bool, DbErr> {
    let db = manager.get_connection();
    let query = Query::select()
        .expr(Expr::value(1))
        .from(Alias::new(table))
        .limit(1)
        .to_owned();
    Ok(db
        .query_one_raw(db.get_database_backend().build(&query))
        .await?
        .is_none())
}

async fn add_column(
    manager: &SchemaManager<'_>,
    table: &str,
    column: ColumnDef,
) -> Result<(), DbErr> {
    let mut column = column;
    manager
        .alter_table(
            Table::alter()
                .table(Alias::new(table))
                .add_column(&mut column)
                .to_owned(),
        )
        .await
}

/// The renamed tables take the places of the empty ones made under their
/// new names in this run.
async fn take_renamed_tables(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    for (old, new) in [
        ("playlist_items", "media_file_playlists"),
        ("media_cover_arts", "media_cover_art"),
    ] {
        if !manager.has_table(old).await? {
            continue;
        }
        if manager.has_table(new).await? {
            if !is_empty(manager, new).await? {
                return Err(DbErr::Custom(format!(
                    "pre-release {old} cannot replace {new}, which has rows"
                )));
            }
            manager
                .drop_table(Table::drop().table(Alias::new(new)).to_owned())
                .await?;
        }
        manager
            .rename_table(
                Table::rename()
                    .table(Alias::new(old), Alias::new(new))
                    .to_owned(),
            )
            .await?;
    }
    if manager
        .has_column("media_file_playlists", "file_id")
        .await?
    {
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("media_file_playlists"))
                    .rename_column(Alias::new("file_id"), Alias::new("media_file_id"))
                    .to_owned(),
            )
            .await?;
    }
    Ok(())
}

/// No build wrote to these tables; one with rows is left as it is.
async fn drop_unwritten_tables(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    for table in ["user_logs", "smart_playlists"] {
        if manager.has_table(table).await? && is_empty(manager, table).await? {
            manager
                .drop_table(Table::drop().table(Alias::new(table)).to_owned())
                .await?;
        }
    }
    Ok(())
}

async fn settle_media_files(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    if !manager.has_column("media_files", "cover_art_id").await? {
        // SQLite adds a column with a reference only through its own syntax.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE media_files ADD COLUMN cover_art_id integer NULL \
                 REFERENCES media_cover_art (id) ON DELETE CASCADE ON UPDATE CASCADE",
            )
            .await?;
    }
    if !manager.has_column("media_files", "sample_rate").await? {
        add_column(
            manager,
            "media_files",
            ColumnDef::new(Alias::new("sample_rate"))
                .integer()
                .not_null()
                .default(0)
                .to_owned(),
        )
        .await?;
    }
    if !manager.has_column("media_files", "duration").await? {
        add_column(
            manager,
            "media_files",
            ColumnDef::new(Alias::new("duration"))
                .double()
                .not_null()
                .default(0.0)
                .to_owned(),
        )
        .await?;
    }
    Ok(())
}

/// The analysis features today's migration 3 makes.
fn analysis_features() -> Vec<String> {
    let mut features: Vec<String> = [
        "rms",
        "zcr",
        "energy",
        "spectral_centroid",
        "spectral_flatness",
        "spectral_slope",
        "spectral_rolloff",
        "spectral_spread",
        "spectral_skewness",
        "spectral_kurtosis",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    features.extend((0..12).map(|index| format!("chroma{index}")));
    features.push("perceptual_spread".to_owned());
    features.push("perceptual_sharpness".to_owned());
    features.extend((0..24).map(|index| format!("perceptual_loudness{index}")));
    features.extend((0..13).map(|index| format!("mfcc{index}")));
    features
}

async fn settle_analysis(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    for old in ["sample_rate", "duration"] {
        if manager.has_column("media_analysis", old).await? {
            manager
                .alter_table(
                    Table::alter()
                        .table(Alias::new("media_analysis"))
                        .drop_column(Alias::new(old))
                        .to_owned(),
                )
                .await?;
        }
    }
    for feature in analysis_features() {
        if !manager.has_column("media_analysis", &feature).await? {
            add_column(
                manager,
                "media_analysis",
                ColumnDef::new(Alias::new(feature.as_str()))
                    .double()
                    .null()
                    .to_owned(),
            )
            .await?;
        }
    }
    Ok(())
}

/// Pre-release scans could link a file to the same artist twice (splitting
/// "quickly, quickly" gave "quickly" twice). A link says the same once as
/// twice, and the sync migrations give each link an identity made from the
/// pair, so one of each pair is kept.
async fn drop_repeated_links(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let db = manager.get_connection();
    for (table, parent) in [
        ("media_file_artists", "artist_id"),
        ("media_file_albums", "album_id"),
    ] {
        if !manager.has_table(table).await? {
            continue;
        }
        db.execute_unprepared(&format!(
            "DELETE FROM {table} WHERE id NOT IN \
             (SELECT min(id) FROM {table} GROUP BY media_file_id, {parent})"
        ))
        .await?;
    }
    Ok(())
}

/// The group an artist or album is listed under, as the scanner gave it
/// when this migration was written: the first letter of the name in Latin
/// letters, upper case, or `#`.
fn group_name(name: &str) -> String {
    let first = deunicode::deunicode(name).chars().next().unwrap_or('#');
    if first.is_lowercase() {
        first.to_ascii_uppercase().to_string()
    } else if first.is_ascii_digit() || !first.is_alphabetic() {
        '#'.to_string()
    } else {
        first.to_string()
    }
}

async fn settle_groups(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let db = manager.get_connection();
    let backend = db.get_database_backend();
    if manager.has_column("playlists", "group").await? {
        let query = Query::update()
            .table(Alias::new("playlists"))
            .value(Alias::new("group"), "")
            .and_where(Expr::col(Alias::new("group")).is_null())
            .to_owned();
        db.execute_raw(backend.build(&query)).await?;
    } else {
        add_column(
            manager,
            "playlists",
            ColumnDef::new(Alias::new("group"))
                .string()
                .not_null()
                .default("")
                .to_owned(),
        )
        .await?;
    }
    for table in ["artists", "albums"] {
        if manager.has_column(table, "group").await? {
            continue;
        }
        add_column(
            manager,
            table,
            ColumnDef::new(Alias::new("group"))
                .string()
                .not_null()
                .default("")
                .to_owned(),
        )
        .await?;
        let rows = db
            .query_all_raw(
                backend.build(
                    &Query::select()
                        .columns([Alias::new("id"), Alias::new("name")])
                        .from(Alias::new(table))
                        .to_owned(),
                ),
            )
            .await?;
        for row in rows {
            let id: i32 = row.try_get("", "id")?;
            let name: String = row.try_get("", "name")?;
            let query = Query::update()
                .table(Alias::new(table))
                .value(Alias::new("group"), group_name(&name))
                .and_where(Expr::col(Alias::new("id")).eq(id))
                .to_owned();
            db.execute_raw(backend.build(&query)).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::group_name;

    #[test]
    fn groups_follow_the_first_letter_in_latin_letters() {
        assert_eq!(group_name("abba"), "A");
        assert_eq!(group_name("Ärzte"), "A");
        assert_eq!(group_name("周杰伦"), "Z");
        assert_eq!(group_name("1975"), "#");
        assert_eq!(group_name(""), "#");
    }
}
