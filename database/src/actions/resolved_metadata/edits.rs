//! Manual album groups and artist identity redirects for the owning library.

use std::collections::BTreeSet;

use anyhow::Result;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, FromQueryResult, TransactionTrait,
};
use uuid::Uuid;

use crate::{
    error::ExpectedError,
    sync::{
        SeaOrmSyncStore,
        model::{Collection, ObjectKey, author_source_on, metadata_anchor_id, metadata_edit_id},
    },
};

use super::statement;

pub const PAGE_SIZE: usize = 50;

/// A selectable edition in the album manager.
#[derive(Debug, FromQueryResult)]
pub struct ReleaseEntry {
    pub id: String,
    pub title: String,
    pub version: Option<String>,
    pub catalog_number: Option<String>,
    pub release_date: Option<String>,
    pub release_type: Option<String>,
    pub directory: String,
    pub group_id: Option<String>,
    pub group_name: Option<String>,
    pub needs_review: bool,
    pub available: bool,
}

/// A selectable canonical artist in the artist manager.
#[derive(Debug, FromQueryResult)]
pub struct ArtistEntry {
    pub id: String,
    pub name: String,
    pub musicbrainz_id: Option<String>,
}

/// A named group owned by this catalog.
#[derive(Debug, FromQueryResult)]
pub struct GroupEntry {
    pub id: String,
    pub name: String,
}

/// Remove a group assignment, create a group, or join an existing group.
#[derive(Debug)]
pub enum AlbumGroupEdit {
    Remove,
    Create(String),
    Join(Uuid),
}

fn pattern(query: &str) -> String {
    let query = query
        .trim()
        .replace('!', "!!")
        .replace('%', "!%")
        .replace('_', "!_");
    format!("%{query}%")
}

/// Return one page of editions. Group members remain editable when their files are absent.
pub async fn releases(
    db: &DatabaseConnection,
    query: &str,
    group: Option<Uuid>,
    offset: u32,
) -> Result<Vec<ReleaseEntry>> {
    super::ensure_tracks_current(db).await?;
    let filter = pattern(query);
    let group_filter = if group.is_some() {
        "a.group_id = ?1"
    } else {
        "?1 IS NULL"
    };
    // The group-specific plan can use the membership index directly.
    let rows = ReleaseEntry::find_by_statement(statement(
        format!("SELECT a.id, current.title, current.version, current.catalog_number, current.release_date, current.release_type,
                a.directory_path AS directory, a.group_id, g.name AS group_name, a.needs_review,
                current.track_count > 0 AS available
         FROM music_metadata_albums a
         LEFT JOIN music_metadata_album_groups g ON g.id = a.group_id
         JOIN music_metadata_current_album_titles current ON current.id = a.id
         WHERE {group_filter}
           AND (current.title LIKE ?2 ESCAPE '!' OR a.directory_path LIKE ?2 ESCAPE '!'
                OR current.version LIKE ?2 ESCAPE '!' OR current.catalog_number LIKE ?2 ESCAPE '!')
         ORDER BY title COLLATE NOCASE, a.directory_path, a.id LIMIT ?3 OFFSET ?4"),
        [
            group.map(|id| id.to_string()).into(),
            filter.into(),
            page_limit(),
            offset.into(),
        ],
    ))
    .all(db)
    .await?;
    Ok(rows)
}

/// Return one page of existing groups matching a name.
pub async fn album_groups(
    db: &DatabaseConnection,
    query: &str,
    offset: u32,
) -> Result<Vec<GroupEntry>> {
    Ok(GroupEntry::find_by_statement(statement(
        "SELECT id, name FROM music_metadata_album_groups
         WHERE name LIKE ? ESCAPE '!' ORDER BY name COLLATE NOCASE, id LIMIT ? OFFSET ?",
        [pattern(query).into(), page_limit(), offset.into()],
    ))
    .all(db)
    .await?)
}

/// Return canonical artists, matching either the main name or any merged name.
pub async fn artists(
    db: &DatabaseConnection,
    query: &str,
    offset: u32,
) -> Result<Vec<ArtistEntry>> {
    Ok(ArtistEntry::find_by_statement(statement(
        "SELECT a.id, a.name, a.musicbrainz_id FROM music_metadata_artists a
         WHERE NOT EXISTS (
            SELECT 1 FROM music_metadata_artist_redirects WHERE source_uuid = a.id)
          AND (a.musicbrainz_id IS NOT NULL OR NOT EXISTS (
            SELECT 1 FROM music_metadata_artists known
            WHERE known.musicbrainz_id IS NOT NULL AND known.name = a.name COLLATE NOCASE)
            OR EXISTS (
                SELECT 1 FROM media_file_artists link
                LEFT JOIN music_metadata_audio track ON track.id = link.track_uuid
                WHERE link.artist_uuid = a.id AND NOT EXISTS (
                    SELECT 1 FROM music_metadata_credits c WHERE c.file_id = track.file_id
                    AND c.scope = 2 AND c.role = 'Primary Artist' AND c.artist_id IS NOT NULL
                    AND c.credited_as = a.name COLLATE NOCASE)))
          AND (a.name LIKE ?1 ESCAPE '!' OR EXISTS (
            SELECT 1 FROM music_metadata_artist_redirects r
            JOIN music_metadata_artists alias ON alias.id = r.source_uuid
            WHERE r.target_uuid = a.id AND alias.name LIKE ?1 ESCAPE '!') OR EXISTS (
            SELECT 1 FROM music_metadata_credits c WHERE c.artist_id = a.id
            AND c.credited_as LIKE ?1 ESCAPE '!'))
         ORDER BY a.name COLLATE NOCASE, a.id LIMIT ?2 OFFSET ?3",
        [pattern(query).into(), page_limit(), offset.into()],
    ))
    .all(db)
    .await?)
}

fn page_limit() -> sea_orm::Value {
    (PAGE_SIZE as i64 + 1).into()
}

/// Update multiple memberships in the same transaction.
pub async fn edit_album_group(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    releases: &[Uuid],
    edit: AlbumGroupEdit,
) -> Result<Option<Uuid>> {
    let releases = selection(releases)?;
    let tx = db.begin().await?;
    store.lock(&tx).await?;

    // Confirm all selected releases still exist before changing any membership.
    for &id in &releases {
        require(
            &tx,
            "SELECT 1 FROM music_metadata_albums WHERE id = ?",
            id,
            "release",
        )
        .await?;
    }
    let group = match edit {
        AlbumGroupEdit::Remove => None,
        AlbumGroupEdit::Join(id) => {
            require(
                &tx,
                "SELECT 1 FROM music_metadata_album_groups WHERE id = ?",
                id,
                "album group",
            )
            .await?;
            Some(id)
        }
        AlbumGroupEdit::Create(name) => {
            let name = name.trim();
            if name.is_empty() || name.chars().count() > 256 {
                return Err(invalid("Album group name must contain 1 to 256 characters"));
            }
            let group = Uuid::new_v4();
            tx.execute_raw(statement(
                "INSERT INTO music_metadata_album_groups (id, name) VALUES (?, ?)",
                [group.to_string().into(), name.to_owned().into()],
            ))
            .await?;
            Some(group)
        }
    };

    let label = if let Some(group) = group {
        let row = tx
            .query_one_raw(statement(
                "SELECT name FROM music_metadata_album_groups WHERE id = ?",
                [group.to_string().into()],
            ))
            .await?
            .ok_or(ExpectedError::NotFound {
                what: "album group",
            })?;
        Some(row.try_get::<String>("", "name")?)
    } else {
        None
    };

    // Publish matching evidence separately from the edition's assignment.
    for release in releases {
        let (edition, generation) = edition_for_edit(&tx, release).await?;
        publish_edition_anchors(&tx, store, edition, release).await?;
        author_edit(
            &tx,
            store,
            "album",
            edition,
            None,
            group,
            label.as_deref(),
            generation,
        )
        .await?;
    }
    tx.execute_unprepared(
        "DELETE FROM music_metadata_albums WHERE group_id IS NULL
         AND NOT EXISTS (SELECT 1 FROM music_metadata_album_field_values WHERE album_id = music_metadata_albums.id)
         AND NOT EXISTS (SELECT 1 FROM music_metadata_local_overlays WHERE album_id = music_metadata_albums.id)
         AND NOT EXISTS (
            SELECT 1 FROM music_metadata_discs WHERE album_id = music_metadata_albums.id);
         DELETE FROM music_metadata_album_groups WHERE NOT EXISTS (
            SELECT 1 FROM music_metadata_albums WHERE group_id = music_metadata_album_groups.id)
            AND NOT EXISTS (SELECT 1 FROM music_metadata_sync_edits
                WHERE kind = 'album' AND target_uuid = music_metadata_album_groups.id);",
    )
    .await?;
    tx.commit().await?;
    Ok(group)
}

async fn edition_for_edit(db: &DatabaseTransaction, release: Uuid) -> Result<(Uuid, i64)> {
    let row = db.query_one_raw(statement(
        "SELECT owner_uuid AS id, generation FROM music_metadata_synced_album_groups WHERE album_id = ?
         ORDER BY generation DESC, owner_uuid LIMIT 1",
        [release.to_string().into()],
    )).await?;
    if let Some(row) = row {
        let generation = row.try_get::<i64>("", "generation")?;
        return Ok((
            super::uuid(&row, "id")?,
            generation
                .checked_add(1)
                .ok_or_else(|| invalid("Edition edit generation overflow"))?,
        ));
    }
    Ok((portable_edition(db, release).await?, 1))
}

pub(in crate::actions) async fn portable_edition(
    db: &impl ConnectionTrait,
    release: Uuid,
) -> Result<Uuid> {
    let row = db.query_one_raw(statement(
        "SELECT COALESCE(
            (SELECT i.edition_uuid FROM music_metadata_release_identity i WHERE i.album_id = ?1
             AND NOT EXISTS (SELECT 1 FROM music_metadata_release_identity other
                             WHERE other.edition_uuid = i.edition_uuid AND other.album_id <> ?1)),
            (SELECT e.edition_uuid FROM music_metadata_edition_matches e
             WHERE e.album_id = ?1 AND e.edition_uuid <> ?1 ORDER BY e.edition_uuid LIMIT 1), ?1) AS id",
        [release.to_string().into()],
    )).await?.ok_or(ExpectedError::NotFound { what: "release" })?;
    super::uuid(&row, "id")
}

pub(in crate::actions) async fn publish_edition_anchors(
    tx: &DatabaseTransaction,
    store: &SeaOrmSyncStore,
    edition: Uuid,
    release: Uuid,
) -> Result<()> {
    // Only current shared tracks can establish identity on another replica.
    let rows = tx.query_all_raw(statement(
        "SELECT t.id FROM music_metadata_discs d JOIN music_metadata_audio t ON t.disc_id = d.id
         WHERE d.album_id = ?1 AND NOT EXISTS (SELECT 1 FROM music_metadata_sync_edits e
            WHERE e.kind = 'anchor' AND e.owner_uuid = ?2 AND e.edition_uuid = t.id)",
        [release.to_string().into(), edition.to_string().into()],
    )).await?;
    for row in rows {
        let track = super::uuid(&row, "id")?;
        author_edit(tx, store, "anchor", edition, Some(track), None, None, 0).await?;
    }
    Ok(())
}

async fn author_edit(
    db: &DatabaseTransaction,
    store: &SeaOrmSyncStore,
    kind: &str,
    owner: Uuid,
    edition: Option<Uuid>,
    target: Option<Uuid>,
    label: Option<&str>,
    generation: i64,
) -> Result<()> {
    let id = if kind == "anchor" {
        metadata_anchor_id(
            owner,
            edition.ok_or_else(|| invalid("Missing shared track"))?,
        )
    } else {
        metadata_edit_id(kind, owner)
    };
    db.execute_raw(statement(
        "INSERT INTO music_metadata_sync_edits (source_uuid, kind, owner_uuid, edition_uuid, target_uuid, label, generation)
         VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(source_uuid) DO UPDATE SET
            target_uuid = excluded.target_uuid, label = excluded.label, generation = excluded.generation",
        [id.to_string().into(), kind.to_owned().into(), owner.to_string().into(),
         edition.map(|id| id.to_string()).into(), target.map(|id| id.to_string()).into(),
         label.map(str::to_owned).into(), generation.into()],
    )).await?;
    author_source_on(
        db,
        store,
        ObjectKey {
            collection: Collection::MetadataEdits,
            id,
        },
    )
    .await
}

/// Merge canonical artists while preserving redirect references on replicas.
pub async fn merge_artists(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    target: Uuid,
    sources: &[Uuid],
) -> Result<()> {
    let sources = selection(sources)?;
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let target = canonical_artist(&tx, target).await?;
    let row = tx
        .query_one_raw(statement(
            "SELECT name FROM music_metadata_artists WHERE id = ?",
            [target.to_string().into()],
        ))
        .await?
        .ok_or(ExpectedError::NotFound { what: "artist" })?;
    let name: String = row.try_get("", "name")?;

    // Resolve old selections before updating any mapping. Repeating a merge is harmless.
    let mut canonical = BTreeSet::new();
    for source in sources {
        let source = canonical_artist(&tx, source).await?;
        if source != target {
            canonical.insert(source);
        }
    }
    for source in canonical {
        author_edit(
            &tx,
            store,
            "artist",
            source,
            None,
            Some(target),
            Some(&name),
            0,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Assign a pending set of tracks to a known edition before the next scan.
pub async fn confirm_release(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    pending: Uuid,
    edition: Uuid,
) -> Result<()> {
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    super::release_group::confirm(&tx, pending, edition).await?;
    tx.commit().await?;
    Ok(())
}

async fn require(db: &impl ConnectionTrait, sql: &str, id: Uuid, what: &'static str) -> Result<()> {
    if db
        .query_one_raw(statement(sql, [id.to_string().into()]))
        .await?
        .is_none()
    {
        return Err(ExpectedError::NotFound { what }.into());
    }
    Ok(())
}

/// Resolve a current or merged artist UUID to its canonical identity.
pub async fn canonical_artist(db: &impl ConnectionTrait, id: Uuid) -> Result<Uuid> {
    let row = db
        .query_one_raw(statement(
            "SELECT COALESCE(r.target_uuid, a.id) AS id FROM music_metadata_artists a
         LEFT JOIN music_metadata_artist_redirects r ON r.source_uuid = a.id WHERE a.id = ?",
            [id.to_string().into()],
        ))
        .await?
        .ok_or(ExpectedError::NotFound { what: "artist" })?;
    super::uuid(&row, "id")
}

fn selection(ids: &[Uuid]) -> Result<BTreeSet<Uuid>> {
    if ids.is_empty() || ids.len() > 512 {
        return Err(invalid("Select between 1 and 512 entries"));
    }
    Ok(ids.iter().copied().collect())
}

fn invalid(detail: &str) -> anyhow::Error {
    ExpectedError::InvalidInput {
        detail: detail.to_owned(),
    }
    .into()
}
