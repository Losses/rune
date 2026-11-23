//! The indexed display values derived from a release's current track evidence.

use anyhow::Result;
use sea_orm::{ConnectionTrait, DatabaseTransaction};
use uuid::Uuid;

use super::statement;

const EDITION_NAMESPACE: Uuid = Uuid::from_u128(0xbe54f834_1696_5fb8_b4f5_4df4a58d1821);

pub(super) async fn refresh_directory(db: &DatabaseTransaction, directory: &str) -> Result<()> {
    let albums = db
        .query_all_raw(statement(
            "SELECT id FROM music_metadata_albums WHERE directory_scope = ?",
            [directory.to_owned().into()],
        ))
        .await?;
    for row in albums {
        refresh(db, super::uuid(&row, "id")?).await?;
    }
    Ok(())
}

pub(super) async fn refresh(db: &DatabaseTransaction, album: Uuid) -> Result<()> {
    let album = album.to_string();
    // Determine the winner per field. The first disc's opening track may have sparse tags.
    let changed = db.execute_raw(statement(
        "WITH evidence AS MATERIALIZED (
            SELECT e.title, e.title_priority, e.version, e.version_priority, e.catalog_number,
                   e.release_date, e.release_type, ROW_NUMBER() OVER (ORDER BY d.disc_number,
                    t.track_number IS NULL, t.track_number, t.file_id) AS track_order
            FROM music_metadata_discs d JOIN music_metadata_audio t ON t.disc_id = d.id
            JOIN music_metadata_album_evidence e ON e.file_id = t.file_id
            WHERE d.album_id = ?1
         )
         INSERT INTO music_metadata_album_summaries
            (album_id, title, version, catalog_number, release_date, release_type, track_count)
         SELECT ?1,
            (SELECT title FROM evidence ORDER BY title_priority, track_order LIMIT 1),
            (SELECT version FROM evidence WHERE version IS NOT NULL ORDER BY version_priority, track_order LIMIT 1),
            (SELECT catalog_number FROM evidence WHERE catalog_number IS NOT NULL ORDER BY track_order LIMIT 1),
            (SELECT release_date FROM evidence WHERE release_date IS NOT NULL ORDER BY track_order LIMIT 1),
            (SELECT release_type FROM evidence WHERE release_type IS NOT NULL ORDER BY track_order LIMIT 1),
            (SELECT count(*) FROM music_metadata_release_tracks t JOIN music_metadata_discs d ON d.id = t.disc_id
             WHERE d.album_id = ?1)
         WHERE EXISTS (SELECT 1 FROM evidence)
         ON CONFLICT(album_id) DO UPDATE SET title = excluded.title, version = excluded.version,
            catalog_number = excluded.catalog_number, release_date = excluded.release_date,
            release_type = excluded.release_type,
            track_count = excluded.track_count",
        [album.clone().into()],
    )).await?;

    // Retain the description of a manually grouped edition while its media is absent.
    if changed.rows_affected() == 0 {
        db.execute_raw(statement(
            "UPDATE music_metadata_album_summaries SET track_count = 0 WHERE album_id = ?",
            [album.clone().into()],
        ))
        .await?;
    } else {
        refresh_identity(db, &album).await?;
    }
    // Apply edits received before this edition was indexed, including explicit removals.
    db.execute_raw(statement(
        "UPDATE music_metadata_albums SET group_id = (
            SELECT target_uuid FROM music_metadata_synced_album_groups WHERE album_id = ?1
            ORDER BY generation DESC, owner_uuid LIMIT 1) WHERE id = ?1",
        [album.clone().into()],
    ))
    .await?;
    db.execute_raw(statement(
        "DELETE FROM music_metadata_dirty_summaries WHERE album_id = ?",
        [album.into()],
    ))
    .await?;
    Ok(())
}

async fn refresh_identity(db: &DatabaseTransaction, album: &str) -> Result<()> {
    // A unique release MBID yields the same edition identity even on disjoint replicas.
    let mbid = db
        .query_one_raw(statement(
            "SELECT min(value) AS id FROM music_metadata_release_keys
         WHERE album_id = ? AND field = 'release' HAVING count(*) = 1",
            [album.to_owned().into()],
        ))
        .await?;
    if let Some(mbid) = mbid {
        let mbid = super::uuid(&mbid, "id")?;
        let edition = Uuid::new_v5(&EDITION_NAMESPACE, mbid.as_bytes());
        db.execute_raw(statement(
            "INSERT INTO music_metadata_release_identity (album_id, musicbrainz_id, edition_uuid)
             VALUES (?, ?, ?) ON CONFLICT(album_id) DO UPDATE SET musicbrainz_id = excluded.musicbrainz_id,
                edition_uuid = excluded.edition_uuid",
            [album.to_owned().into(), mbid.to_string().into(), edition.to_string().into()],
        )).await?;
    } else {
        db.execute_raw(statement(
            "DELETE FROM music_metadata_release_identity WHERE album_id = ?",
            [album.to_owned().into()],
        ))
        .await?;
    }
    Ok(())
}
