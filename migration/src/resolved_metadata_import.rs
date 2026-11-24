//! Populate resolved editions from the catalog already stored on disk.

use std::collections::BTreeMap;

use sea_orm_migration::{
    prelude::*,
    sea_orm::{ConnectionTrait, FromQueryResult},
    sea_query::{InsertStatement, OnConflict, SelectStatement},
};
use uuid::Uuid;

use crate::catalog_identity::{ParentKind, parent};

const BATCH: u64 = 192;

#[derive(FromQueryResult)]
struct FileRow {
    id: i32,
    source_uuid: String,
    directory: String,
    file_name: String,
    last_modified: String,
    sample_rate: i32,
    duration: f64,
    content_hash: Option<String>,
    content_size: Option<i64>,
    album_uuid: Option<String>,
    indexed_track: Option<i32>,
    album_name: Option<String>,
}

#[derive(FromQueryResult)]
struct TagRow {
    file_id: i32,
    meta_key: String,
    meta_value: String,
}

pub(super) async fn import(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let db = manager.get_connection();
    import_album_identities(db).await?;
    let mut after = 0;
    loop {
        let files =
            FileRow::find_by_statement(db.get_database_backend().build(&files_after(after)))
                .all(db)
                .await?;
        let Some(last) = files.last() else {
            break;
        };
        after = last.id;
        import_files(db, files, true).await?;
    }
    cache_album_summaries(db).await?;

    // A later local scan can enrich migrated metadata even when the file bytes did not change.
    let projected = Query::select()
        .column(alias("file_id"))
        .from(alias("music_metadata_audio"))
        .to_owned();
    let invalidate = Query::update()
        .table(alias("media_files"))
        .value(alias("change_token"), Expr::val(None::<String>))
        .and_where(Expr::col(alias("local_available")).eq(true))
        .and_where(Expr::col(alias("id")).in_subquery(projected))
        .to_owned();
    db.execute_raw(db.get_database_backend().build(&invalidate))
        .await?;
    Ok(())
}

/// Project a synchronized track into the catalog without overriding file evidence.
pub async fn project_portable_track(db: &impl ConnectionTrait, track: Uuid) -> Result<(), DbErr> {
    // Portable updates are only authoritative for files that have no local scan.
    let native = Query::select()
        .expr(Expr::val(1))
        .from_as(alias("music_metadata_audio"), alias("native"))
        .and_where(column("native", "file_id").eq(column("file", "id")))
        .and_where(column("native", "portable").eq(0))
        .to_owned();
    let query = files_after(0)
        .and_where(column("file", "source_uuid").eq(track.to_string()))
        .and_where(column("file", "local_available").eq(false))
        .and_where(Expr::not_exists(native))
        .to_owned();
    let Some(file) = FileRow::find_by_statement(db.get_database_backend().build(&query))
        .one(db)
        .await?
    else {
        return Ok(());
    };

    // Replace only source-tag credits. User and provider credits stay intact.
    let cleanup = Query::delete()
        .from_table(alias("music_metadata_credits"))
        .and_where(Expr::col(alias("file_id")).eq(file.id))
        .and_where(Expr::col(alias("source_kind")).eq("audio_tag"))
        .to_owned();
    db.execute_raw(db.get_database_backend().build(&cleanup))
        .await?;
    let genres = Query::delete()
        .from_table(alias("music_metadata_values"))
        .and_where(Expr::col(alias("file_id")).eq(file.id))
        .and_where(Expr::col(alias("scope")).eq(2))
        .and_where(Expr::col(alias("field")).eq("genres"))
        .to_owned();
    db.execute_raw(db.get_database_backend().build(&genres))
        .await?;
    import_files(db, vec![file], false).await
}

fn alias(name: &str) -> Alias {
    Alias::new(name)
}

fn column(table: &str, name: &str) -> Expr {
    Expr::col((alias(table), alias(name)))
}

fn insert(table: &str, columns: &[&str]) -> InsertStatement {
    Query::insert()
        .into_table(alias(table))
        .columns(columns.iter().map(|name| alias(name)))
        .on_conflict(OnConflict::new().do_nothing().to_owned())
        .to_owned()
}

fn replace_file_fields(table: &str, columns: &[&str]) -> InsertStatement {
    insert(table, columns)
        .on_conflict(
            OnConflict::column(alias("file_id"))
                .update_columns(columns.iter().skip(1).map(|name| alias(name)))
                .to_owned(),
        )
        .to_owned()
}

async fn import_album_identities(db: &impl ConnectionTrait) -> Result<(), DbErr> {
    // Copied editions predate the insert trigger, including those without tracks.
    let handles = Query::select()
        .expr(Expr::val("album"))
        .column(alias("id"))
        .from(alias("music_metadata_albums"))
        .and_where(Expr::col(alias("portable_title")).is_not_null())
        .order_by(alias("legacy_id"), Order::Asc)
        .to_owned();
    let mut target = insert("music_metadata_browse_handles", &["kind", "source_uuid"]);
    target
        .select_from(handles)
        .map_err(|error| DbErr::Custom(error.to_string()))?;
    db.execute_raw(db.get_database_backend().build(&target))
        .await?;
    Ok(())
}

async fn cache_album_summaries(db: &impl ConnectionTrait) -> Result<(), DbErr> {
    // Populate the normal browse cache once after all imported tracks are linked.
    let fields = [
        "album_id",
        "title",
        "version",
        "catalog_number",
        "release_date",
        "release_type",
        "track_count",
    ];
    let rows = Query::select()
        .columns(fields.map(alias))
        .from(alias("music_metadata_live_summaries"))
        .and_where(Expr::col(alias("album_id")).is_not_null())
        .to_owned();
    let mut target = insert("music_metadata_album_summaries", &fields);
    target
        .select_from(rows)
        .map_err(|error| DbErr::Custom(error.to_string()))?;

    let backend = db.get_database_backend();
    db.execute_raw(backend.build(&target)).await?;
    for table in [
        "music_metadata_dirty_summaries",
        "music_metadata_dirty_tracks",
    ] {
        db.execute_raw(backend.build(Query::delete().from_table(alias(table))))
            .await?;
    }
    Ok(())
}

fn files_after(after: i32) -> SelectStatement {
    // Membership UUIDs make migration independent of display names and directory availability.
    let mut files = Query::select();
    files
        .columns(
            [
                "id",
                "source_uuid",
                "directory",
                "file_name",
                "last_modified",
                "sample_rate",
                "duration",
                "content_hash",
                "content_size",
            ]
            .map(|name| (alias("file"), alias(name))),
        )
        .column((alias("link"), alias("album_uuid")))
        .expr_as(column("link", "track_number"), alias("indexed_track"))
        .expr_as(column("edition", "portable_title"), alias("album_name"))
        .from_as(alias("media_files"), alias("file"));

    // A file with no source tags still needs an audio identity in the catalog.
    files
        .join_as(
            JoinType::LeftJoin,
            alias("music_metadata_track_album_sources"),
            alias("link"),
            column("file", "source_uuid").eq(column("link", "track_uuid")),
        )
        .join_as(
            JoinType::LeftJoin,
            alias("music_metadata_albums"),
            alias("edition"),
            column("edition", "id").eq(column("link", "album_uuid")),
        )
        .and_where(column("file", "id").gt(after))
        .order_by((alias("file"), alias("id")), Order::Asc)
        .limit(BATCH)
        .to_owned()
}

async fn import_files(
    db: &impl ConnectionTrait,
    files: Vec<FileRow>,
    backfill_tracks: bool,
) -> Result<(), DbErr> {
    // Fetch source evidence in bounded pages and honor its original order.
    let query = Query::select()
        .columns(["file_id", "meta_key", "meta_value"].map(alias))
        .from(alias("music_metadata_source_tags"))
        .and_where(Expr::col(alias("file_id")).is_in(files.iter().map(|file| file.id)))
        .order_by(alias("position"), Order::Asc)
        .order_by(alias("id"), Order::Asc)
        .to_owned();
    let mut by_file = BTreeMap::<i32, BTreeMap<String, Vec<String>>>::new();
    for tag in TagRow::find_by_statement(db.get_database_backend().build(&query))
        .all(db)
        .await?
    {
        by_file
            .entry(tag.file_id)
            .or_default()
            .entry(tag.meta_key)
            .or_default()
            .push(tag.meta_value);
    }

    // Insert parents first, then the normalized fields that refer to them.
    let mut batch = ImportBatch::new(backfill_tracks);
    for file in files {
        let tags = by_file.remove(&file.id).unwrap_or_default();
        batch.push(file, &tags)?;
    }
    for query in batch
        .statements
        .into_iter()
        .chain(batch.tracks.into_iter().flatten())
    {
        db.execute_raw(db.get_database_backend().build(&query))
            .await?;
    }
    insert_chunks(
        db,
        "music_metadata_credits",
        &[
            "file_id",
            "scope",
            "position",
            "credited_as",
            "role",
            "source_kind",
            "source_provider",
        ],
        &batch.credits,
    )
    .await?;
    insert_chunks(
        db,
        "music_metadata_values",
        &[
            "file_id",
            "scope",
            "field",
            "qualifier",
            "position",
            "value",
        ],
        &batch.genres,
    )
    .await
}

async fn insert_chunks<const N: usize>(
    db: &impl ConnectionTrait,
    table: &str,
    columns: &[&str; N],
    data: &[[Expr; N]],
) -> Result<(), DbErr> {
    // Repeatable fields are batched separately to keep SQLite bind sizes bounded.
    for rows in data.chunks(BATCH as usize) {
        let mut query = insert(table, columns);
        for row in rows {
            query.values_panic(row.clone());
        }
        db.execute_raw(db.get_database_backend().build(&query))
            .await?;
    }
    Ok(())
}

struct ImportBatch {
    statements: [InsertStatement; 6],
    tracks: Option<[InsertStatement; 2]>,
    credits: Vec<[Expr; 7]>,
    genres: Vec<[Expr; 6]>,
}

impl ImportBatch {
    fn new(backfill_tracks: bool) -> Self {
        Self {
            statements: [
                insert("music_metadata_albums", &["id", "directory_path"]),
                insert("music_metadata_discs", &["id", "album_id", "disc_number"]),
                insert("music_metadata_recordings", &["id"]),
                replace_file_fields(
                    "music_metadata_audio",
                    &[
                        "file_id",
                        "id",
                        "disc_id",
                        "recording_id",
                        "recording_content_hash",
                        "recording_content_size",
                        "title",
                        "track_title_key",
                        "track_number",
                        "disc_number",
                        "duration_ms",
                        "import_date",
                        "sample_rate",
                        "portable",
                    ],
                ),
                replace_file_fields(
                    "music_metadata_album_evidence",
                    &[
                        "file_id",
                        "title",
                        "tag_title",
                        "title_priority",
                        "version_priority",
                    ],
                ),
                replace_file_fields(
                    "music_metadata_disc_evidence",
                    &["file_id", "title_priority", "directory_path"],
                ),
            ],
            tracks: backfill_tracks.then(|| {
                [
                    insert(
                        "music_metadata_release_tracks",
                        &["id", "disc_id", "recording_id", "track_number", "title"],
                    ),
                    insert(
                        "music_metadata_track_assets",
                        &["track_id", "file_id", "start_ms"],
                    ),
                ]
            }),
            credits: Vec::new(),
            genres: Vec::new(),
        }
    }

    fn push(&mut self, file: FileRow, tags: &BTreeMap<String, Vec<String>>) -> Result<(), DbErr> {
        // Keep the old edition identity, even when media is unavailable.
        let source = |key: &str| {
            tags.get(key)
                .and_then(|values| values.last())
                .filter(|value| !value.trim().is_empty())
        };
        let audio = parse_id(&file.source_uuid)?;
        let title = source("track_title")
            .or_else(|| source("title"))
            .unwrap_or(&file.file_name);
        let album_title = file
            .album_name
            .as_ref()
            .or_else(|| source("album"))
            .map_or("", String::as_str);
        let album = match file.album_uuid.as_deref() {
            Some(id) => parse_id(id)?,
            None => Uuid::new_v5(
                &parent(ParentKind::Album, album_title),
                file.directory.as_bytes(),
            ),
        };
        let (disc_number, track_number) = positions(&file, tags);
        let medium = disc_number.unwrap_or(1);
        let disc = Uuid::new_v5(&album, &medium.to_be_bytes());
        let recording = Uuid::new_v5(&audio, b"recording");

        // Preserve the portable file UUID and the exact legacy album membership.
        let [
            albums,
            discs,
            recordings,
            files,
            album_evidence,
            disc_evidence,
        ] = &mut self.statements;
        albums.values_panic([
            Expr::val(album.to_string()),
            Expr::val(file.directory.clone()),
        ]);
        discs.values_panic([
            Expr::val(disc.to_string()),
            Expr::val(album.to_string()),
            Expr::val(medium),
        ]);
        recordings.values_panic([Expr::val(recording.to_string())]);
        let title_key = title
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        if let Some([tracks, assets]) = &mut self.tracks {
            // Whole-file tracks use the same edition slot UUID as runtime reconciliation.
            let slot = track_number.map_or_else(
                || format!("file:{audio}"),
                |number| format!("slot:{number}:{title_key}"),
            );
            let track = Uuid::new_v5(&disc, slot.as_bytes());
            let track_recording = Uuid::new_v5(&track, b"release-track-recording").to_string();
            recordings.values_panic([Expr::val(track_recording.clone())]);
            tracks.values_panic([
                Expr::val(track.to_string()),
                Expr::val(disc.to_string()),
                Expr::val(track_recording),
                Expr::val(track_number),
                Expr::val(title),
            ]);
            assets.values_panic([
                Expr::val(track.to_string()),
                Expr::val(file.id),
                Expr::val(0),
            ]);
        }

        // Both initial migration and subsequent sync updates retain the per-file evidence.
        files.values_panic([
            Expr::val(file.id),
            Expr::val(audio.to_string()),
            Expr::val(disc.to_string()),
            Expr::val(recording.to_string()),
            Expr::val(file.content_hash),
            Expr::val(file.content_size),
            Expr::val(title),
            Expr::val(title_key),
            Expr::val(track_number),
            Expr::val(disc_number),
            Expr::val((file.duration.max(0.0) * 1000.0).round() as i64),
            Expr::val(file.last_modified),
            Expr::val(file.sample_rate),
            Expr::val(1),
        ]);

        // Keep source spellings in credits independently of the indexed artist links.
        album_evidence.values_panic([
            Expr::val(file.id),
            Expr::val(album_title),
            Expr::val(
                file.album_name
                    .as_ref()
                    .or_else(|| source("album"))
                    .cloned(),
            ),
            Expr::val(0),
            Expr::val(0),
        ]);
        disc_evidence.values_panic([Expr::val(file.id), Expr::val(0), Expr::val(file.directory)]);
        for (scope, key) in [(0, "album_artist"), (2, "artist")] {
            for (position, name) in tags.get(key).into_iter().flatten().enumerate() {
                let position = <i32 as TryFrom<usize>>::try_from(position)
                    .map_err(|error| DbErr::Custom(error.to_string()))?;
                self.credits.push([
                    Expr::val(file.id),
                    Expr::val(scope),
                    Expr::val(position),
                    Expr::val(name),
                    Expr::val("Primary Artist"),
                    Expr::val("audio_tag"),
                    Expr::val(""),
                ]);
            }
        }
        // Preserve the historical double-backslash genre separator.
        for (position, genre) in tags
            .get("genre")
            .into_iter()
            .flatten()
            .flat_map(|value| value.split("\\\\"))
            .filter(|name| !name.is_empty())
            .enumerate()
        {
            let position = <i32 as TryFrom<usize>>::try_from(position)
                .map_err(|error| DbErr::Custom(error.to_string()))?;
            self.genres.push([
                Expr::val(file.id),
                Expr::val(2),
                Expr::val("genres"),
                Expr::val(""),
                Expr::val(position),
                Expr::val(genre),
            ]);
        }
        Ok(())
    }
}

fn parse_id(value: &str) -> Result<Uuid, DbErr> {
    value
        .parse()
        .map_err(|error: uuid::Error| DbErr::Custom(format!("catalog UUID {value}: {error}")))
}

fn positions(file: &FileRow, tags: &BTreeMap<String, Vec<String>>) -> (Option<u32>, Option<u32>) {
    let number = |key: &str| {
        tags.get(key)
            .and_then(|values| values.last())
            .and_then(|value| value.split('/').next())
            .and_then(|value| value.trim().parse::<u32>().ok())
    };
    let indexed = file
        .indexed_track
        .and_then(|value| <u32 as TryFrom<i32>>::try_from(value).ok());
    let disc = number("disc_number")
        .or_else(|| indexed.map(|value| value / 1000))
        .filter(|value| *value > 0);
    let track = number("track_number").or_else(|| indexed.map(|value| value % 1000));
    (disc, track)
}
