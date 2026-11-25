//! Local album, disc, and audio metadata catalog.

mod artist_identity;
mod columns;
mod credits;
pub(super) mod cue;
mod details;
pub(crate) mod lifecycle;
pub(super) mod portable;
pub(crate) mod queries;
mod recording_identity;
mod recordings;
pub use recordings::{
    recording_metadata, register_recording_source, replace_recording_credit_source,
    update_recording_source_quality,
};
mod release_group;
pub(super) mod summary;
pub(crate) mod synced;
mod tracks;
pub use tracks::{disc_track_ids, release_track_metadata};

use std::collections::BTreeMap;

use anyhow::Result;
use metadata::{
    model::{
        AlbumMetadata, AlbumSource, AudioEncodingInfo, AudioMetadata, Credit, DiscMetadata,
        FieldSource, MetadataBase, MetadataSourceKind,
    },
    resolve::LocalMetadata,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseTransaction, ExprTrait, FromQueryResult, QueryResult,
    SqliteTransactionMode, TransactionOptions, TransactionSession, TransactionTrait, Value,
    sea_query::{
        Alias, Asterisk, BinOper, Expr, InsertStatement, IntoTableRef, LikeExpr, OnConflict, Order,
        Query, SelectStatement,
    },
};
use uuid::Uuid;

use crate::entities::{
    media_files,
    music_metadata::{
        album_evidence, album_summaries, albums as orm_albums, audio as orm_audio, dirty_summaries,
        dirty_tracks as orm_dirty_tracks, disc_evidence, discs as orm_discs,
    },
};
use details::Selection;

type Winners = BTreeMap<&'static str, i32>;

/// The scope of a credit contributed by one indexed audio file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum CreditScope {
    Album = 0,
    Disc = 1,
    Audio = 2,
}

/// Replace the credits contributed by one provider within a write transaction.
pub async fn replace_credit_source(
    db: &DatabaseTransaction,
    track_id: Uuid,
    scope: CreditScope,
    source: MetadataSourceKind,
    provider: &str,
    values: &[Credit],
) -> Result<bool> {
    let tx = db.begin().await?;
    let updated = credits::replace_source(&tx, track_id, scope, source, provider, values).await?;
    tx.commit().await?;
    Ok(updated)
}

const AUDIO_COLUMNS: &[&[&str]] = &[
    &[
        "file_id",
        "id",
        "disc_id",
        "recording_id",
        "recording_mbid",
        "recording_isrc",
        "recording_content_hash",
        "recording_content_size",
        "release_track_mbid",
        "title",
        "track_title_key",
        "duration_ms",
        "import_date",
    ],
    columns::COMMON,
    columns::AUDIO,
    columns::ENCODING,
];
const ALBUM_COLUMNS: &[&[&str]] = &[
    &[
        "file_id",
        "title",
        "tag_title",
        "title_priority",
        "version_priority",
    ],
    columns::COMMON,
    columns::ALBUM,
];
const DISC_COLUMNS: &[&[&str]] = &[
    &["file_id", "title", "title_priority"],
    columns::COMMON,
    columns::DISC,
];

fn evidence_upsert(
    table: impl IntoTableRef,
    columns: &[&[&str]],
    values: Vec<Value>,
) -> Result<InsertStatement> {
    let names = columns.iter().flat_map(|group| group.iter().copied());
    let updates = names
        .clone()
        .filter(|name| !matches!(*name, "file_id" | "import_date"));
    let mut query = Query::insert();
    query
        .into_table(table)
        .columns(names.map(Alias::new))
        .values(values.into_iter().map(Expr::val))?
        .on_conflict(
            OnConflict::column(Alias::new("file_id"))
                .update_columns(updates.map(Alias::new))
                .to_owned(),
        );
    Ok(query)
}

pub(super) fn column(column: impl ColumnTrait) -> Expr {
    Expr::col(column.as_column_ref())
}

pub(super) fn nocase(expr: Expr) -> Expr {
    expr.binary(BinOper::Custom("COLLATE"), Expr::custom_keyword("NOCASE"))
}

pub(super) fn like_pattern(pattern: &str) -> LikeExpr {
    LikeExpr::new(pattern).escape('!')
}

pub(super) async fn query_models<T: FromQueryResult>(
    db: &impl ConnectionTrait,
    query: &SelectStatement,
) -> Result<Vec<T>> {
    db.query_all(query)
        .await?
        .iter()
        .map(|row| Ok(T::from_query_result(row, "")?))
        .collect()
}

pub(super) fn scalar(query: SelectStatement) -> Expr {
    Expr::SubQuery(None, Box::new(query.into()))
}

pub(super) async fn contains(db: &impl ConnectionTrait, file_id: i32) -> Result<bool> {
    use crate::entities::music_metadata::native_audio;
    Ok(db
        .query_one(
            Query::select()
                .expr(Expr::val(1))
                .from(native_audio::Entity)
                .and_where(native_audio::Column::FileId.eq(file_id)),
        )
        .await?
        .is_some())
}

pub(super) async fn store(
    db: &DatabaseTransaction,
    file: &media_files::Model,
    metadata: &LocalMetadata,
    moved_from: Option<&str>,
) -> Result<()> {
    store_on(db, file, metadata, moved_from, false).await?;
    Ok(())
}

async fn store_on(
    db: &DatabaseTransaction,
    file: &media_files::Model,
    metadata: &LocalMetadata,
    moved_from: Option<&str>,
    managed: bool,
) -> Result<String> {
    let file_id = file.id;
    let source_uuid = file.source_uuid.parse()?;
    let (album, disc, audio) = (&metadata.album, &metadata.disc, &metadata.audio);
    let AlbumSource::Local { directory_path } = &album.source else {
        anyhow::bail!("expected local album source");
    };
    let keys = release_group::ReleaseKeys::from_track(metadata);
    let managed_key = if managed {
        Some(release_group::managed_key(db, source_uuid, &keys).await?)
    } else {
        None
    };
    let album_id = release_group::album_id(
        db,
        directory_path,
        managed_key.as_deref(),
        file_id,
        &keys,
        moved_from,
    )
    .await?;
    let scope = if managed {
        db.query_one(
            Query::select()
                .column(orm_albums::Column::DirectoryScope)
                .from(orm_albums::Entity)
                .and_where(Expr::col(orm_albums::Column::Id).eq(Expr::val(album_id.to_string()))),
        )
        .await?
        .ok_or_else(|| anyhow::anyhow!("indexed edition disappeared"))?
        .try_get::<String>("", "directory_scope")?
    } else {
        directory_path.clone()
    };
    let disc_id = ensure_disc(db, album_id, disc.disc_number).await?;
    tracks::prepare_move(db, file_id, &disc_id).await?;
    let ids = metadata::model::RecordingIdentifiers {
        musicbrainz_id: external_uuid(&audio.base, "musicbrainz:recording"),
        isrc: audio.isrc.as_deref().and_then(recording_identity::isrc),
    };
    let recording_id = recording_identity::prepare_audio(
        db,
        source_uuid,
        &ids,
        (file.content_hash.as_deref(), file.content_size),
    )
    .await?;

    // The conflict clause preserves the original import date.
    let mut values = vec![
        file_id.into(),
        source_uuid.to_string().into(),
        disc_id.into(),
        recording_id.to_string().into(),
        ids.musicbrainz_id.map(|id| id.to_string()).into(),
        ids.isrc.into(),
        file.content_hash.clone().into(),
        file.content_size.into(),
        external_uuid(&audio.base, "musicbrainz:release-track")
            .map(|id| id.to_string())
            .into(),
        audio.title.clone().into(),
        tracks::title_key(&audio.title).into(),
        columns::bind_duration(audio.base.duration_ms)?,
        jiff::Timestamp::now().to_string().into(),
    ];
    columns::bind_common(&audio.base, &mut values)?;
    columns::bind_audio(audio, &mut values)?;
    columns::bind_encoding(
        audio
            .encoding
            .as_ref()
            .unwrap_or(&AudioEncodingInfo::default()),
        &mut values,
    )?;
    db.execute(&evidence_upsert(orm_audio::Entity, AUDIO_COLUMNS, values)?)
        .await?;
    // Replace all evidence for this track within the caller's scan transaction.
    let mut values = vec![
        file_id.into(),
        release_group::album_title(metadata).into(),
        metadata
            .diagnostics
            .raw_tags
            .get("ALBUM")
            .and_then(|values| values.last())
            .cloned()
            .into(),
        release_group::title_priority(metadata).into(),
        release_group::version_priority(metadata).into(),
    ];
    columns::bind_common(&album.base, &mut values)?;
    columns::bind_album(album, &mut values)?;
    db.execute(&evidence_upsert(
        album_evidence::Entity,
        ALBUM_COLUMNS,
        values,
    )?)
    .await?;
    let mut values = vec![
        file_id.into(),
        disc.title.clone().into(),
        title_rank(disc).into(),
    ];
    columns::bind_common(&disc.base, &mut values)?;
    columns::bind_disc(disc, &mut values)?;
    db.execute(&evidence_upsert(
        disc_evidence::Entity,
        DISC_COLUMNS,
        values,
    )?)
    .await?;
    use crate::entities::music_metadata::native_audio;
    db.execute(
        Query::insert()
            .into_table(native_audio::Entity)
            .columns([native_audio::Column::FileId])
            .values_panic([Expr::val(file_id)])
            .on_conflict(OnConflict::new().do_nothing().to_owned()),
    )
    .await?;
    release_group::store(db, file_id, album_id, keys).await?;
    release_group::queue(db, &scope).await?;
    credits::store(db, file_id, metadata).await?;
    details::store(db, file_id, metadata).await?;
    Ok(scope)
}

pub(in crate::actions) async fn store_download(
    db: &DatabaseTransaction,
    file: &media_files::Model,
    metadata: &LocalMetadata,
) -> Result<()> {
    let scope = store_on(db, file, metadata, None, true).await?;
    recording_identity::reconcile(db).await?;
    release_group::refresh_scope(db, &scope).await
}

pub(crate) async fn reconcile_recordings(db: &impl ConnectionTrait) -> Result<()> {
    recording_identity::reconcile(db).await?;
    tracks::reconcile(db).await?;
    summary::reconcile(db).await
}

pub(super) async fn ensure_tracks_current(db: &sea_orm::DatabaseConnection) -> Result<()> {
    if db
        .query_one(
            Query::select()
                .expr(Expr::val(1))
                .from(orm_dirty_tracks::Entity)
                .union(
                    sea_orm::sea_query::UnionType::All,
                    Query::select()
                        .expr(Expr::val(1))
                        .from(dirty_summaries::Entity)
                        .to_owned(),
                )
                .limit(1),
        )
        .await?
        .is_some()
    {
        // Acquire the writer reservation before reading the queues so SQLite can wait for sync.
        let tx = db
            .begin_with_options(TransactionOptions {
                sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
                ..Default::default()
            })
            .await?;
        tracks::reconcile(&tx).await?;
        summary::reconcile(&tx).await?;
        tx.commit().await?;
    }
    Ok(())
}

pub(crate) async fn project_synced_track(
    db: &DatabaseTransaction,
    track: Uuid,
) -> Result<(), sea_orm::DbErr> {
    let key = track.to_string();
    let previous = lifecycle::audio_disc(db, &key).await?;
    recording_identity::invalidate_stale_file(db, &key)
        .await
        .map_err(|error| sea_orm::DbErr::Custom(error.to_string()))?;
    if let Some(file) = synced::project(db, track).await? {
        lifecycle::finish_portable_projection(db, file, previous)
            .await
            .map_err(|error| sea_orm::DbErr::Custom(error.to_string()))?;
    }
    Ok(())
}

pub(super) async fn track_album_directory(
    db: &impl ConnectionTrait,
    file_id: i32,
) -> Result<Option<String>> {
    let row = db
        .query_one(
            Query::select()
                .column(orm_albums::Column::DirectoryPath.as_column_ref())
                .from(orm_audio::Entity)
                .inner_join(
                    orm_discs::Entity,
                    column(orm_discs::Column::Id).eq(column(orm_audio::Column::DiscId)),
                )
                .inner_join(
                    orm_albums::Entity,
                    column(orm_albums::Column::Id).eq(column(orm_discs::Column::AlbumId)),
                )
                .and_where(orm_audio::Column::FileId.eq(file_id)),
        )
        .await?;
    row.map(|row| Ok(row.try_get("", "directory_path")?))
        .transpose()
}

async fn ensure_disc(db: &DatabaseTransaction, album: Uuid, number: u32) -> Result<String> {
    // A disc ID is stable for its assigned release and medium number.
    let id = Uuid::new_v5(&album, &number.to_be_bytes()).to_string();
    db.execute(
        Query::insert()
            .into_table(orm_discs::Entity)
            .columns([
                orm_discs::Column::Id,
                orm_discs::Column::AlbumId,
                orm_discs::Column::DiscNumber,
            ])
            .values_panic([
                Expr::val(id.clone()),
                Expr::val(album.to_string()),
                Expr::val(i64::from(number)),
            ])
            .on_conflict(OnConflict::new().do_nothing().to_owned()),
    )
    .await?;
    Ok(id)
}

pub(super) async fn reconcile_releases(
    db: &sea_orm::DatabaseConnection,
    store: &crate::sync::SeaOrmSyncStore,
    cancellation: Option<&tokio_util::sync::CancellationToken>,
) -> Result<()> {
    release_group::reconcile(db, store, cancellation).await
}

/// Retrieve resolved track metadata using its catalog UUID.
pub async fn audio_metadata(db: &impl TransactionTrait, id: Uuid) -> Result<Option<AudioMetadata>> {
    let tx = db.begin().await?;
    recording_identity::reconcile(&tx).await?;
    let recording: Expr = Expr::case(
        queries::matches_content(),
        column(orm_audio::Column::RecordingId),
    )
    .finally(Expr::val(None::<String>))
    .into();
    let row = tx
        .query_one(
            Query::select()
                .column((orm_audio::Entity, Asterisk))
                .column(orm_discs::Column::AlbumId.as_column_ref())
                .expr_as(recording, "current_recording_id")
                .from(orm_audio::Entity)
                .inner_join(
                    orm_discs::Entity,
                    column(orm_discs::Column::Id).eq(column(orm_audio::Column::DiscId)),
                )
                .left_join(
                    media_files::Entity,
                    column(media_files::Column::Id).eq(column(orm_audio::Column::FileId)),
                )
                .and_where(orm_audio::Column::Id.eq(id.to_string())),
        )
        .await?;
    let Some(row) = row else {
        tx.commit().await?;
        return Ok(None);
    };

    // Load the scalar row and its multi-valued fields from the same read snapshot.
    let file_id = row.try_get("", "file_id")?;
    let mut audio = AudioMetadata {
        base: MetadataBase {
            id,
            duration_ms: columns::duration(&row)?,
            import_date: row.try_get("", "import_date")?,
            ..Default::default()
        },
        title: row.try_get("", "title")?,
        disc_id: Some(uuid(&row, "disc_id")?),
        recording_id: optional_uuid(&row, "current_recording_id")?,
        album_id: Some(uuid(&row, "album_id")?),
        ..Default::default()
    };

    let mut winners = Winners::new();
    columns::fill_common(&mut audio.base, &row, file_id, &mut winners)?;
    columns::fill_audio(&mut audio, &row, file_id, &mut winners)?;
    let mut encoding = AudioEncodingInfo::default();
    columns::fill_encoding(&mut encoding, &row, file_id, &mut winners)?;
    audio.encoding = (encoding != AudioEncodingInfo::default()).then_some(encoding);
    let lists = details::load(
        &tx,
        Selection::Audio(file_id),
        None,
        &mut audio.base,
        &winners,
    )
    .await?;
    audio.fingerprints = lists.fingerprints;
    tx.commit().await?;
    Ok(Some(audio))
}

/// Retrieve a disc, resolving title priority across all indexed tracks.
pub async fn disc_metadata(db: &impl TransactionTrait, id: Uuid) -> Result<Option<DiscMetadata>> {
    let tx = db.begin().await?;
    tracks::reconcile(&tx).await?;
    let rows = tx
        .query_all(
            Query::select()
                .column((disc_evidence::Entity, Asterisk))
                .columns(
                    [orm_discs::Column::AlbumId, orm_discs::Column::DiscNumber]
                        .map(|c| c.as_column_ref()),
                )
                .expr_as(column(orm_audio::Column::Id), "audio_id")
                .column(orm_audio::Column::DurationMs.as_column_ref())
                .from(orm_audio::Entity)
                .inner_join(
                    orm_discs::Entity,
                    column(orm_discs::Column::Id).eq(column(orm_audio::Column::DiscId)),
                )
                .inner_join(
                    disc_evidence::Entity,
                    column(disc_evidence::Column::FileId).eq(column(orm_audio::Column::FileId)),
                )
                .and_where(orm_audio::Column::DiscId.eq(id.to_string()))
                .order_by_expr(orm_audio::Column::TrackNumber.is_null(), Order::Asc)
                .order_by(orm_audio::Column::TrackNumber.as_column_ref(), Order::Asc)
                .order_by(orm_audio::Column::FileId.as_column_ref(), Order::Asc),
        )
        .await?;
    let Some(first) = rows.first() else {
        tx.commit().await?;
        return Ok(None);
    };

    // Other shared disc fields use the first track as their source.
    let first_file: i32 = first.try_get("", "file_id")?;
    let mut disc = DiscMetadata {
        base: MetadataBase {
            id,
            ..Default::default()
        },
        album_id: uuid(first, "album_id")?,
        disc_number: first.try_get::<i64>("", "disc_number")?.try_into()?,
        track_count: None,
        ..Default::default()
    };
    let mut winners = Winners::new();
    columns::fill_common(&mut disc.base, first, first_file, &mut winners)?;

    let mut best = i32::MAX;
    let mut duration = Duration::default();
    for row in &rows {
        let file_id = row.try_get("", "file_id")?;
        columns::fill_disc(&mut disc, row, file_id, &mut winners)?;
        let priority: i32 = row.try_get("", "title_priority")?;
        if priority < best {
            best = priority;
            disc.title = row.try_get("", "title")?;
            winners.insert("title", file_id);
        }
    }

    let logical_tracks = tracks::duration_rows(&tx, id, false).await?;
    disc.track_count = Some(logical_tracks.len().try_into()?);
    for row in &logical_tracks {
        duration.add(row)?;
    }

    details::load(
        &tx,
        Selection::Disc(id),
        Some(first_file),
        &mut disc.base,
        &winners,
    )
    .await?;
    duration.finish(&mut disc.base);
    tx.commit().await?;
    Ok(Some(disc))
}

fn title_rank(disc: &DiscMetadata) -> i32 {
    const ALIASES: &[&str] = &[
        "DISCSUBTITLE",
        "SETSUBTITLE",
        "SET SUBTITLE",
        "TSST",
        "DISCNAME",
    ];
    if disc.title.is_none() {
        return ALIASES.len() as i32 + 1;
    }
    disc.base
        .provenance
        .as_ref()
        .and_then(|p| p.get("title"))
        .into_iter()
        .flatten()
        .filter_map(|source| source.key.as_deref())
        .filter_map(|key| {
            let key = ["TXXX:", "----:com.apple.iTunes:", "freeform:"]
                .iter()
                .find_map(|prefix| key.strip_prefix(prefix))
                .unwrap_or(key);
            ALIASES
                .iter()
                .position(|alias| key.eq_ignore_ascii_case(alias))
        })
        .min()
        .unwrap_or(ALIASES.len()) as i32
}

#[derive(Default)]
struct Duration {
    total: Option<u64>,
    sources: Vec<FieldSource>,
}

impl Duration {
    fn add(&mut self, row: &QueryResult) -> Result<()> {
        let Some(duration) = columns::duration(row)? else {
            return Ok(());
        };
        self.total = Some(self.total.unwrap_or_default().saturating_add(duration));
        self.sources.push(FieldSource {
            kind: MetadataSourceKind::Aggregate,
            key: None,
            path: None,
            pattern: None,
            entity_id: Some(uuid(row, "audio_id")?),
            confidence: None,
        });
        Ok(())
    }

    fn finish(self, base: &mut MetadataBase) {
        base.duration_ms = self.total;
        if !self.sources.is_empty() {
            base.provenance
                .get_or_insert_default()
                .insert("duration_ms".to_owned(), self.sources);
        }
    }
}

fn uuid(row: &QueryResult, column: &str) -> Result<Uuid> {
    Ok(row.try_get::<String>("", column)?.parse()?)
}

fn optional_uuid(row: &QueryResult, column: &str) -> Result<Option<Uuid>> {
    Ok(row
        .try_get::<Option<String>>("", column)?
        .map(|value| value.parse())
        .transpose()?)
}

pub(super) fn external_uuid(base: &MetadataBase, namespace: &str) -> Option<Uuid> {
    let mut values = base
        .identifiers
        .as_ref()?
        .get(namespace)?
        .iter()
        .flat_map(|value| value.split([';', ',', '\0']))
        .filter_map(|value| Uuid::parse_str(value.trim()).ok())
        .filter(|id| !id.is_nil());
    let first = values.next()?;
    values.all(|id| id == first).then_some(first)
}

/// Retrieve a release and its current indexed totals.
pub async fn album_metadata(db: &impl TransactionTrait, id: Uuid) -> Result<Option<AlbumMetadata>> {
    let tx = db.begin().await?;
    tracks::reconcile(&tx).await?;
    let rows = tx
        .query_all(
            Query::select()
                .column((album_evidence::Entity, Asterisk))
                .expr_as(column(orm_audio::Column::Id), "audio_id")
                .column(orm_audio::Column::DurationMs.as_column_ref())
                .column(orm_discs::Column::DiscNumber.as_column_ref())
                .columns(
                    [
                        orm_albums::Column::DirectoryPath,
                        orm_albums::Column::NeedsReview,
                    ]
                    .map(|c| c.as_column_ref()),
                )
                .expr_as(column(album_summaries::Column::Title), "display_title")
                .expr_as(column(album_summaries::Column::Version), "display_version")
                .expr_as(
                    column(album_summaries::Column::CatalogNumber),
                    "display_catalog_number",
                )
                .expr_as(
                    column(album_summaries::Column::ReleaseDate),
                    "display_release_date",
                )
                .expr_as(
                    column(album_summaries::Column::ReleaseType),
                    "display_release_type",
                )
                .expr_as(
                    column(album_summaries::Column::TrackCount),
                    "display_track_count",
                )
                .from(orm_albums::Entity)
                .inner_join(
                    orm_discs::Entity,
                    column(orm_discs::Column::AlbumId).eq(column(orm_albums::Column::Id)),
                )
                .inner_join(
                    orm_audio::Entity,
                    column(orm_audio::Column::DiscId).eq(column(orm_discs::Column::Id)),
                )
                .inner_join(
                    album_evidence::Entity,
                    column(album_evidence::Column::FileId).eq(column(orm_audio::Column::FileId)),
                )
                .left_join(
                    album_summaries::Entity,
                    column(album_summaries::Column::AlbumId)
                        .eq(column(orm_albums::Column::Id))
                        .and(Expr::not_exists(
                            Query::select()
                                .column(dirty_summaries::Column::AlbumId.as_column_ref())
                                .from(dirty_summaries::Entity)
                                .and_where(
                                    column(dirty_summaries::Column::AlbumId)
                                        .eq(column(orm_albums::Column::Id)),
                                )
                                .to_owned(),
                        )),
                )
                .and_where(orm_albums::Column::Id.eq(id.to_string()))
                .order_by(orm_discs::Column::DiscNumber.as_column_ref(), Order::Asc)
                .order_by_expr(orm_audio::Column::TrackNumber.is_null(), Order::Asc)
                .order_by(orm_audio::Column::TrackNumber.as_column_ref(), Order::Asc)
                .order_by(orm_audio::Column::FileId.as_column_ref(), Order::Asc),
        )
        .await?;

    let Some(first) = rows.first() else {
        tx.commit().await?;
        return Ok(None);
    };
    let mut album = empty_album(id, first)?;
    let mut winners = Winners::from([("title", first.try_get("", "file_id")?)]);
    let mut title_priority: i32 = first.try_get("", "title_priority")?;
    let mut version_priority = i32::MAX;
    let mut duration = Duration::default();
    let mut last_disc = None;
    let mut disc_count = 0;

    // Fill missing scalars from the next track with evidence in release order.
    for row in &rows {
        let file_id = row.try_get("", "file_id")?;
        let priority: i32 = row.try_get("", "title_priority")?;
        if priority < title_priority {
            title_priority = priority;
            album.title = row.try_get("", "title")?;
            winners.insert("title", file_id);
        }
        columns::fill_common(&mut album.base, row, file_id, &mut winners)?;
        columns::fill_album(&mut album, row, file_id, &mut winners)?;
        let priority: i32 = row.try_get("", "version_priority")?;
        if priority < version_priority
            && let Some(version) = row.try_get::<Option<String>>("", "version")?
        {
            version_priority = priority;
            album.base.version = Some(version);
            winners.insert("version", file_id);
        }
        let disc: i64 = row.try_get("", "disc_number")?;
        if last_disc.replace(disc) != Some(disc) {
            disc_count += 1;
        }
    }

    let logical_tracks = tracks::duration_rows(&tx, id, true).await?;
    for row in &logical_tracks {
        duration.add(row)?;
    }

    let lists = details::load(&tx, Selection::Album(id), None, &mut album.base, &winners).await?;
    album.labels = lists.album.labels;
    album.extra_resources = lists.album.extra_resources;
    album.release_secondary_types = lists.album.secondary_types;
    album.disc_count = album.disc_count.or(Some(disc_count));
    album.total_tracks = Some(logical_tracks.len().try_into()?);
    if let Some(title) = first.try_get::<Option<String>>("", "display_title")? {
        album.title = title;
        album.base.version = first.try_get("", "display_version")?;
        album.catalog_number = first.try_get("", "display_catalog_number")?;
        album.base.release_date = first.try_get("", "display_release_date")?;
        album.release_type = first
            .try_get::<Option<String>>("", "display_release_type")?
            .as_deref()
            .and_then(metadata::model::ReleaseType::from_label);
        album.total_tracks = first
            .try_get::<Option<i64>>("", "display_track_count")?
            .map(u32::try_from)
            .transpose()?;
    }
    duration.finish(&mut album.base);
    tx.commit().await?;
    Ok(Some(album))
}

fn empty_album(id: Uuid, row: &QueryResult) -> Result<AlbumMetadata> {
    Ok(AlbumMetadata {
        base: MetadataBase {
            id,
            ..Default::default()
        },
        title: row.try_get("", "title")?,
        needs_review: row.try_get("", "needs_review")?,
        source: AlbumSource::Local {
            directory_path: row.try_get("", "directory_path")?,
        },
        extra_resources: None,
        barcode: None,
        catalog_number: None,
        labels: None,
        release_country: None,
        release_type: None,
        release_secondary_types: Default::default(),
        release_status: None,

        // Totals, release flags, and loudness are resolved after reading the evidence.
        disc_count: None,
        total_tracks: None,
        declared_track_count: None,
        replaygain_album_gain_db: None,
        replaygain_album_peak: None,
        replaygain_reference_lufs: None,
    })
}

/// List releases grouped by the resolved album identity.
pub async fn album_ids(db: &impl ConnectionTrait) -> Result<Vec<Uuid>> {
    db.query_all(
        Query::select()
            .column(orm_albums::Column::Id)
            .from(orm_albums::Entity)
            .order_by(orm_albums::Column::DirectoryPath, Order::Asc)
            .order_by(orm_albums::Column::Id, Order::Asc),
    )
    .await?
    .iter()
    .map(|row| uuid(row, "id"))
    .collect()
}

/// List persisted discs in release order.
pub async fn disc_ids(db: &impl ConnectionTrait, album: Uuid) -> Result<Vec<Uuid>> {
    db.query_all(
        Query::select()
            .column(orm_discs::Column::Id)
            .from(orm_discs::Entity)
            .and_where(Expr::col(orm_discs::Column::AlbumId).eq(Expr::val(album.to_string())))
            .order_by(orm_discs::Column::DiscNumber, Order::Asc),
    )
    .await?
    .iter()
    .map(|row| uuid(row, "id"))
    .collect()
}
