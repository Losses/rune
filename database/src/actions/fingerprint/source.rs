//! Assign fingerprint and similarity values to their existing source rows.

use crate::entities::{media_file_fingerprint, media_file_similarity, media_files};
use crate::sync::bindings::apply;
use crate::sync::model::{
    Collection, Fingerprint, MAX_PORTABLE_FINGERPRINT_BYTES, Payload, ScoreValue, Similarity,
};
use anyhow::{Result, ensure};
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, Set};
use tag_editor::music_brainz::fingerprint::Configuration;

/// A computed fingerprint and the provenance used to derive its identity.
pub struct ComputedFingerprint {
    pub algorithm: String,
    pub configuration: String,
    pub format_version: i32,
    pub bytes: Vec<u8>,
    pub is_duplicated: i32,
}

/// A similarity result and the fingerprint bytes and provenance used to compute it.
pub struct ComputedSimilarity<'a> {
    pub first_fingerprint: &'a [u8],
    pub second_fingerprint: &'a [u8],
    pub algorithm: String,
    pub configuration: String,
    pub format_version: i32,
    pub score: f32,
}

/// Commit a computed fingerprint only if its observed source file is unchanged.
pub async fn assign_computed_fingerprint(
    db: &sea_orm::DatabaseConnection,
    store: &crate::sync::SeaOrmSyncStore,
    observed: &media_files::Model,
    computed: ComputedFingerprint,
) -> Result<()> {
    let ComputedFingerprint {
        algorithm,
        configuration,
        format_version,
        bytes,
        is_duplicated,
    } = computed;
    use crate::sync::model::ObjectKey;
    use sea_orm::TransactionTrait;
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let file = checked_file(&tx, observed).await?;
    let file_id = file.hlc_uuid.parse()?;
    let id = migration::catalog_identity::fingerprint(
        file_id,
        Some(&algorithm),
        Some(&configuration),
        Some(format_version),
    );
    if bytes.len() > MAX_PORTABLE_FINGERPRINT_BYTES {
        let key = ObjectKey {
            collection: Collection::Fingerprints,
            id,
        };
        if store
            .get_on(&tx, &key)
            .await?
            .is_some_and(|record| record.value.is_some())
        {
            store.author_on(&tx, key, None).await?;
        }
        let existing = media_file_fingerprint::Entity::find()
            .filter(media_file_fingerprint::Column::HlcUuid.eq(id.to_string()))
            .one(&tx)
            .await?;
        let mut row: media_file_fingerprint::ActiveModel =
            existing.map(Into::into).unwrap_or_default();
        row.hlc_uuid = Set(id.to_string());
        row.track_uuid = Set(file_id.to_string());
        row.fingerprint = Set(bytes);
        row.is_duplicated = Set(is_duplicated);
        row.source_hash = Set(file.content_hash);
        row.source_size = Set(file.content_size);
        row.algorithm = Set(Some(algorithm));
        row.configuration = Set(Some(configuration));
        row.format_version = Set(Some(format_version));
        row.save(&tx).await?;
        log::debug!("keeping oversized fingerprint local for {}", file.id);
        tx.commit().await?;
        return Ok(());
    }
    if let Some(existing) = media_file_fingerprint::Entity::find()
        .filter(media_file_fingerprint::Column::HlcUuid.eq(id.to_string()))
        .one(&tx)
        .await?
        && existing.fingerprint.len() > MAX_PORTABLE_FINGERPRINT_BYTES
    {
        let mut update: media_file_fingerprint::ActiveModel = existing.into();
        update.fingerprint = Set(bytes.clone());
        update.update(&tx).await?;
    }
    let value = Some(Payload::Fingerprint(Fingerprint {
        file_id,
        source_hash: file.content_hash,
        source_size: file.content_size,
        algorithm: Some(algorithm),
        configuration: Some(configuration),
        format_version: Some(format_version),
        bytes,
        is_duplicated,
    }));
    let key = ObjectKey {
        collection: Collection::Fingerprints,
        id,
    };
    let record = match store.get_on(&tx, &key).await? {
        Some(record) if record.value == value => record,
        _ => store.author_on(&tx, key, value).await?,
    };
    apply(&tx, &record.key, record.value.as_ref()).await?;
    tx.commit().await?;
    Ok(())
}

/// Commit an ordered similarity result if both file observations remain current.
pub async fn assign_computed_similarity(
    db: &sea_orm::DatabaseConnection,
    store: &crate::sync::SeaOrmSyncStore,
    first: &media_files::Model,
    second: &media_files::Model,
    computed: ComputedSimilarity<'_>,
) -> Result<()> {
    let ComputedSimilarity {
        first_fingerprint,
        second_fingerprint,
        algorithm,
        configuration,
        format_version,
        score,
    } = computed;
    use crate::sync::model::ObjectKey;
    use sea_orm::TransactionTrait;
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let first = checked_file(&tx, first).await?;
    let second = checked_file(&tx, second).await?;
    let first_row = current_fingerprint(&tx, &first)
        .await?
        .ok_or_else(|| anyhow::anyhow!("first fingerprint disappeared"))?;
    let second_row = current_fingerprint(&tx, &second)
        .await?
        .ok_or_else(|| anyhow::anyhow!("second fingerprint disappeared"))?;
    ensure!(
        first_row.fingerprint == first_fingerprint
            && second_row.fingerprint == second_fingerprint
            && fingerprint_status(&first_row, &first) == SourceStatus::Verified
            && fingerprint_status(&second_row, &second) == SourceStatus::Verified,
        "fingerprint changed during comparison"
    );
    let first_file_id = first.hlc_uuid.parse()?;
    let second_file_id = second.hlc_uuid.parse()?;
    let id = migration::catalog_identity::similarity(
        first_file_id,
        second_file_id,
        Some(&algorithm),
        Some(&configuration),
        Some(format_version),
    );
    let value = Some(Payload::Similarity(Similarity {
        first_file_id,
        second_file_id,
        first_source_hash: first.content_hash,
        first_source_size: first.content_size,
        second_source_hash: second.content_hash,
        second_source_size: second.content_size,
        algorithm: Some(algorithm),
        configuration: Some(configuration),
        format_version: Some(format_version),
        score: ScoreValue(if score == 0.0 { 0 } else { score.to_bits() }),
    }));
    let key = ObjectKey {
        collection: Collection::Similarities,
        id,
    };
    let record = match store.get_on(&tx, &key).await? {
        Some(record) if record.value == value => record,
        _ => store.author_on(&tx, key, value).await?,
    };
    apply(&tx, &record.key, record.value.as_ref()).await?;
    tx.commit().await?;
    Ok(())
}

async fn checked_file(
    tx: &DatabaseTransaction,
    observed: &media_files::Model,
) -> Result<media_files::Model> {
    let current = media_files::Entity::find_by_id(observed.id)
        .one(tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("source file disappeared"))?;
    ensure!(
        current.hlc_uuid == observed.hlc_uuid
            && current.last_modified == observed.last_modified
            && current.content_hash == observed.content_hash
            && current.content_size == observed.content_size
            && current.local_available,
        "source changed during analysis"
    );
    Ok(current)
}

/// The source/provenance state used by computation and presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceStatus {
    Verified,
    Unknown,
    Unsupported,
    Changed,
}

pub fn fingerprint_status(
    row: &media_file_fingerprint::Model,
    file: &media_files::Model,
) -> SourceStatus {
    let Ok(file_id) = file.hlc_uuid.parse() else {
        return SourceStatus::Changed;
    };
    let expected = migration::catalog_identity::fingerprint(
        file_id,
        row.algorithm.as_deref(),
        row.configuration.as_deref(),
        row.format_version,
    );
    if row.track_uuid != file.hlc_uuid || row.hlc_uuid != expected.to_string() {
        return SourceStatus::Changed;
    }
    if row.algorithm.is_none()
        || row.configuration.is_none()
        || row.format_version.is_none()
        || row.source_hash.is_none()
        || row.source_size.is_none()
    {
        return SourceStatus::Unknown;
    }
    if row.algorithm.as_deref() != Some("chromaprint")
        || row.configuration.as_deref() != Some("rusty-chromaprint-0.3.0/default")
        || row.format_version != Some(1)
    {
        return SourceStatus::Unsupported;
    }
    if row.source_hash != file.content_hash || row.source_size != file.content_size {
        return SourceStatus::Changed;
    }
    SourceStatus::Verified
}

/// Select exactly the supported fingerprint identity for this source file.
pub async fn current_fingerprint<C: sea_orm::ConnectionTrait>(
    db: &C,
    file: &media_files::Model,
) -> Result<Option<media_file_fingerprint::Model>> {
    let file_id: uuid::Uuid = file.hlc_uuid.parse()?;
    let id = migration::catalog_identity::fingerprint(
        file_id,
        Some("chromaprint"),
        Some("rusty-chromaprint-0.3.0/default"),
        Some(1),
    );
    let rows = media_file_fingerprint::Entity::find()
        .filter(media_file_fingerprint::Column::HlcUuid.eq(id.to_string()))
        .all(db)
        .await?;
    ensure!(rows.len() <= 1, "duplicate current fingerprint identity");
    if let Some(row) = rows.first() {
        ensure!(
            row.track_uuid == file.hlc_uuid,
            "fingerprint identity points to different file"
        );
    }
    Ok(rows.into_iter().next())
}

/// Set the shared duplicate mark using the existing source row and an atomic assignment.
pub async fn assign_duplicate_mark(
    db: &sea_orm::DatabaseConnection,
    store: &crate::sync::SeaOrmSyncStore,
    file_id: i32,
    mark: i32,
) -> Result<()> {
    use crate::sync::model::ObjectKey;
    use sea_orm::TransactionTrait;
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let file = media_files::Entity::find_by_id(file_id)
        .one(&tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing file"))?;
    let row = current_fingerprint(&tx, &file)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing supported fingerprint"))?;
    ensure!(
        fingerprint_status(&row, &file) == SourceStatus::Verified,
        "fingerprint is unavailable"
    );
    let uuid = file.hlc_uuid.parse()?;
    let id = migration::catalog_identity::fingerprint(
        uuid,
        row.algorithm.as_deref(),
        row.configuration.as_deref(),
        row.format_version,
    );
    ensure!(
        row.hlc_uuid == id.to_string(),
        "fingerprint source identity requires correction"
    );
    if row.fingerprint.len() > MAX_PORTABLE_FINGERPRINT_BYTES {
        let mut update: media_file_fingerprint::ActiveModel = row.into();
        update.is_duplicated = Set(mark);
        update.update(&tx).await?;
        log::debug!("keeping oversized fingerprint local for {file_id}");
        tx.commit().await?;
        return Ok(());
    }
    let value = Some(Payload::Fingerprint(Fingerprint {
        file_id: uuid,
        source_hash: row.source_hash,
        source_size: row.source_size,
        algorithm: row.algorithm,
        configuration: row.configuration,
        format_version: row.format_version,
        bytes: row.fingerprint,
        is_duplicated: mark,
    }));
    let key = ObjectKey {
        collection: Collection::Fingerprints,
        id,
    };
    let record = match store.get_on(&tx, &key).await? {
        Some(record) if record.value == value => record,
        _ => store.author_on(&tx, key, value).await?,
    };
    apply(&tx, &record.key, record.value.as_ref()).await?;
    tx.commit().await?;
    Ok(())
}

/// Return retained fingerprint rows that cannot currently be used for matching.
pub async fn unavailable_fingerprints(
    db: &sea_orm::DatabaseConnection,
) -> Result<Vec<(i32, SourceStatus)>> {
    let rows = media_file_fingerprint::Entity::find().all(db).await?;
    let mut unavailable = Vec::new();
    for row in rows {
        let status = match media_files::Entity::find()
            .filter(media_files::Column::HlcUuid.eq(row.track_uuid.clone()))
            .one(db)
            .await?
        {
            Some(file) => fingerprint_status(&row, &file),
            None => SourceStatus::Changed,
        };
        if status != SourceStatus::Verified {
            unavailable.push((row.id, status));
        }
    }
    Ok(unavailable)
}

pub fn default_similarity_configuration() -> String {
    let config = Configuration::default();
    format!(
        "rusty-chromaprint-0.3.0/{}",
        blake3::hash(format!("{config:?}").as_bytes()).to_hex()
    )
}

pub fn similarity_status(
    row: &media_file_similarity::Model,
    first: &media_files::Model,
    second: &media_files::Model,
) -> SourceStatus {
    let (Ok(first_id), Ok(second_id)) = (first.hlc_uuid.parse(), second.hlc_uuid.parse()) else {
        return SourceStatus::Changed;
    };
    let expected = migration::catalog_identity::similarity(
        first_id,
        second_id,
        row.algorithm.as_deref(),
        row.configuration.as_deref(),
        row.format_version,
    );
    if row.first_track_uuid != first.hlc_uuid
        || row.second_track_uuid != second.hlc_uuid
        || row.hlc_uuid != expected.to_string()
    {
        return SourceStatus::Changed;
    }
    if row.algorithm.is_none()
        || row.configuration.is_none()
        || row.format_version.is_none()
        || row.first_source_hash.is_none()
        || row.first_source_size.is_none()
        || row.second_source_hash.is_none()
        || row.second_source_size.is_none()
    {
        return SourceStatus::Unknown;
    }
    if row.algorithm.as_deref() != Some("chromaprint-match")
        || row.format_version != Some(1)
        || row.configuration.as_deref() != Some(default_similarity_configuration().as_str())
    {
        return SourceStatus::Unsupported;
    }
    if row.first_source_hash != first.content_hash
        || row.first_source_size != first.content_size
        || row.second_source_hash != second.content_hash
        || row.second_source_size != second.content_size
    {
        return SourceStatus::Changed;
    }
    SourceStatus::Verified
}

pub async fn unavailable_similarities(
    db: &sea_orm::DatabaseConnection,
) -> Result<Vec<(i32, SourceStatus)>> {
    let rows = media_file_similarity::Entity::find().all(db).await?;
    let mut unavailable = Vec::new();
    for row in rows {
        let first = media_files::Entity::find()
            .filter(media_files::Column::HlcUuid.eq(row.first_track_uuid.clone()))
            .one(db)
            .await?;
        let second = media_files::Entity::find()
            .filter(media_files::Column::HlcUuid.eq(row.second_track_uuid.clone()))
            .one(db)
            .await?;
        let status = match (first, second) {
            (Some(first), Some(second)) => similarity_status(&row, &first, &second),
            _ => SourceStatus::Changed,
        };
        if status != SourceStatus::Verified {
            unavailable.push((row.id, status));
        }
    }
    Ok(unavailable)
}
