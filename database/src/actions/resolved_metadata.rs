//! Local album, disc, and audio metadata catalog.

mod artist_identity;
mod columns;
mod credits;
pub(super) mod cue;
mod details;
pub mod edits;
pub mod fields;
pub mod overlays;
mod recording_identity;
mod recordings;
pub use recordings::{
    recording_metadata, register_recording_source, replace_recording_credit_source,
    update_recording_source_quality,
};
mod release_group;
mod summary;
mod tracks;
pub use tracks::{disc_track_ids, release_track_metadata, set_track_layout};

use std::{collections::BTreeMap, sync::LazyLock};

use anyhow::Result;
use metadata::{
    model::{
        AlbumMetadata, AlbumSource, AudioEncodingInfo, AudioMetadata, Credit, DiscMetadata,
        FieldSource, MetadataBase, MetadataSourceKind,
    },
    resolve::LocalMetadata,
};
use sea_orm::{
    ConnectionTrait, DatabaseTransaction, DbBackend, QueryResult, Statement, TransactionSession,
    TransactionTrait, Value,
};
use uuid::Uuid;

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

/// Set or clear a confirmed artist link for a credit on an indexed file.
pub async fn link_credit_artist(
    db: &impl ConnectionTrait,
    track_id: Uuid,
    scope: CreditScope,
    credit: &Credit,
    artist_id: Option<Uuid>,
) -> Result<bool> {
    credits::link(db, track_id, scope, credit, artist_id).await
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

static AUDIO_UPSERT: LazyLock<String> = LazyLock::new(|| {
    upsert(
        "music_metadata_audio",
        &[
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
        ],
    )
});
static ALBUM_UPSERT: LazyLock<String> = LazyLock::new(|| {
    upsert(
        "music_metadata_album_evidence",
        &[
            &["file_id", "title", "title_priority", "version_priority"],
            columns::COMMON,
            columns::ALBUM,
        ],
    )
});
static DISC_UPSERT: LazyLock<String> = LazyLock::new(|| {
    upsert(
        "music_metadata_disc_evidence",
        &[
            &["file_id", "title", "title_priority"],
            columns::COMMON,
            columns::DISC,
        ],
    )
});

fn upsert(table: &str, columns: &[&[&str]]) -> String {
    let columns = columns.concat();
    let names = columns.join(", ");
    let parameters = vec!["?"; columns.len()].join(", ");
    let updates = columns
        .iter()
        .filter(|&&column| !matches!(column, "file_id" | "import_date"))
        .map(|column| format!("{column} = excluded.{column}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "INSERT INTO {table} ({names}) VALUES ({parameters}) ON CONFLICT(file_id) DO UPDATE SET {updates}"
    )
}

fn statement(sql: impl Into<String>, values: impl IntoIterator<Item = Value>) -> Statement {
    Statement::from_sql_and_values(DbBackend::Sqlite, sql, values)
}

pub(super) async fn contains(db: &impl ConnectionTrait, file_id: i32) -> Result<bool> {
    Ok(db
        .query_one_raw(statement(
            "SELECT 1 FROM music_metadata_audio WHERE file_id = ?",
            [file_id.into()],
        ))
        .await?
        .is_some())
}

pub(super) async fn store(
    db: &DatabaseTransaction,
    file: &crate::entities::media_files::Model,
    metadata: &LocalMetadata,
    moved_from: Option<&str>,
) -> Result<()> {
    store_on(db, file, metadata, moved_from, false).await?;
    Ok(())
}

async fn store_on(
    db: &DatabaseTransaction,
    file: &crate::entities::media_files::Model,
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
        db.query_one_raw(statement(
            "SELECT directory_scope FROM music_metadata_albums WHERE id = ?",
            [album_id.to_string().into()],
        ))
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
    let recording_id = recording_identity::prepare_audio(db, source_uuid, &ids).await?;

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
    db.execute_raw(statement(AUDIO_UPSERT.as_str(), values))
        .await?;
    // Replace all evidence for this track within the caller's scan transaction.
    let mut values = vec![
        file_id.into(),
        release_group::album_title(metadata).into(),
        release_group::title_priority(metadata).into(),
        release_group::version_priority(metadata).into(),
    ];
    columns::bind_common(&album.base, &mut values)?;
    columns::bind_album(album, &mut values)?;
    db.execute_raw(statement(ALBUM_UPSERT.as_str(), values))
        .await?;
    let mut values = vec![
        file_id.into(),
        disc.title.clone().into(),
        title_rank(disc).into(),
    ];
    columns::bind_common(&disc.base, &mut values)?;
    columns::bind_disc(disc, &mut values)?;
    db.execute_raw(statement(DISC_UPSERT.as_str(), values))
        .await?;
    release_group::store(db, file_id, album_id, keys).await?;
    release_group::queue(db, &scope).await?;
    credits::store(db, file_id, metadata).await?;
    details::store(db, file_id, metadata).await?;
    Ok(scope)
}

pub(in crate::actions) async fn store_download(
    db: &DatabaseTransaction,
    file: &crate::entities::media_files::Model,
    metadata: &LocalMetadata,
) -> Result<()> {
    let scope = store_on(db, file, metadata, None, true).await?;
    recording_identity::reconcile(db).await?;
    release_group::refresh_scope(db, &scope).await
}

pub(crate) async fn reconcile_recordings(db: &impl ConnectionTrait) -> Result<()> {
    recording_identity::reconcile(db).await?;
    tracks::reconcile(db).await
}

pub(super) async fn ensure_tracks_current(db: &sea_orm::DatabaseConnection) -> Result<()> {
    if db
        .query_one_raw(statement(
            "SELECT 1 FROM music_metadata_dirty_tracks LIMIT 1",
            [],
        ))
        .await?
        .is_some()
    {
        let tx = db.begin().await?;
        tracks::reconcile(&tx).await?;
        tx.commit().await?;
    }
    Ok(())
}

pub(super) async fn track_album_directory(
    db: &impl ConnectionTrait,
    file_id: i32,
) -> Result<Option<String>> {
    let row = db
        .query_one_raw(statement(
            "SELECT r.directory_path FROM music_metadata_audio t
         JOIN music_metadata_discs d ON d.id = t.disc_id
         JOIN music_metadata_albums r ON r.id = d.album_id WHERE t.file_id = ?",
            [file_id.into()],
        ))
        .await?;
    row.map(|row| Ok(row.try_get("", "directory_path")?))
        .transpose()
}

async fn ensure_disc(db: &DatabaseTransaction, album: Uuid, number: u32) -> Result<String> {
    // A disc ID is stable for its assigned release and medium number.
    let id = Uuid::new_v5(&album, &number.to_be_bytes()).to_string();
    db.execute_raw(statement(
        "INSERT INTO music_metadata_discs(id, album_id, disc_number) VALUES (?, ?, ?) ON CONFLICT DO NOTHING",
        [id.clone().into(), album.to_string().into(), i64::from(number).into()],
    )).await?;
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
    let row = tx
        .query_one_raw(statement(
            "SELECT a.*, d.album_id, valid.recording_id AS current_recording_id FROM music_metadata_audio a
         JOIN music_metadata_discs d ON d.id = a.disc_id
         LEFT JOIN music_metadata_current_recording_audio valid ON valid.file_id = a.file_id WHERE a.id = ?",
            [id.to_string().into()],
        ))
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
    overlays::apply(&tx, overlays::OverlayTarget::Audio(id), &mut audio).await?;
    tx.commit().await?;
    Ok(Some(audio))
}

/// Retrieve a disc, resolving title priority across all indexed tracks.
pub async fn disc_metadata(db: &impl TransactionTrait, id: Uuid) -> Result<Option<DiscMetadata>> {
    let tx = db.begin().await?;
    tracks::reconcile(&tx).await?;
    let rows = tx
        .query_all_raw(statement(
            "SELECT e.*, d.album_id, d.disc_number, a.id AS audio_id, a.duration_ms
         FROM music_metadata_audio a
         JOIN music_metadata_discs d ON d.id = a.disc_id
         JOIN music_metadata_disc_evidence e ON e.file_id = a.file_id
         WHERE a.disc_id = ?
         ORDER BY a.track_number IS NULL, a.track_number, a.file_id",
            [id.to_string().into()],
        ))
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
    overlays::apply(&tx, overlays::OverlayTarget::Disc(id), &mut disc).await?;
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
        .query_all_raw(statement(
            "SELECT e.*, a.id AS audio_id, a.duration_ms, d.disc_number, r.directory_path, r.group_id, r.needs_review,
                    s.title AS display_title, s.version AS display_version,
                    s.catalog_number AS display_catalog_number, s.release_date AS display_release_date,
                    s.release_type AS display_release_type,
                    s.track_count AS display_track_count
         FROM music_metadata_albums r
         JOIN music_metadata_discs d ON d.album_id = r.id
         JOIN music_metadata_audio a ON a.disc_id = d.id
         JOIN music_metadata_album_evidence e ON e.file_id = a.file_id
         LEFT JOIN music_metadata_album_summaries s ON s.album_id = r.id AND NOT EXISTS (
            SELECT 1 FROM music_metadata_dirty_summaries dirty WHERE dirty.album_id = r.id)
         WHERE r.id = ?
         ORDER BY d.disc_number, a.track_number IS NULL, a.track_number, a.file_id",
            [id.to_string().into()],
        ))
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
    fields::apply(&tx, &mut album).await?;
    overlays::apply(&tx, overlays::OverlayTarget::Album(id), &mut album).await?;
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
        group_id: row
            .try_get::<Option<String>>("", "group_id")?
            .map(|id| id.parse())
            .transpose()?,
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
    db.query_all_raw(Statement::from_string(
        DbBackend::Sqlite,
        "SELECT id FROM music_metadata_albums ORDER BY directory_path, id",
    ))
    .await?
    .iter()
    .map(|row| uuid(row, "id"))
    .collect()
}

/// List persisted discs in release order.
pub async fn disc_ids(db: &impl ConnectionTrait, album: Uuid) -> Result<Vec<Uuid>> {
    db.query_all_raw(statement(
        "SELECT id FROM music_metadata_discs WHERE album_id = ? ORDER BY disc_number",
        [album.to_string().into()],
    ))
    .await?
    .iter()
    .map(|row| uuid(row, "id"))
    .collect()
}
