use sea_orm_migration::{prelude::*, sea_orm::ConnectionTrait};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }

    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(include_str!("resolved_metadata.sql"))
            .await?;

        // Keep the shared track anchor valid when a scan moves media between editions.
        for (name, event) in [("insert", "INSERT"), ("move", "UPDATE OF disc_id")] {
            manager.get_connection().execute_unprepared(&format!(
                "CREATE TRIGGER music_metadata_album_anchor_{name} AFTER {event} ON music_metadata_audio
                 BEGIN
                    INSERT INTO music_metadata_album_track_anchors (track_uuid, album_id)
                    SELECT NEW.id, album_id FROM music_metadata_discs WHERE id = NEW.disc_id
                    ON CONFLICT(track_uuid) DO UPDATE SET album_id = excluded.album_id
                    WHERE album_id <> excluded.album_id;
                 END;"
            )).await?;
        }

        // Remove parents as soon as their last contribution is deleted or moves.
        for (name, event) in [("delete", "DELETE"), ("move", "UPDATE OF disc_id")] {
            manager.get_connection().execute_unprepared(&format!(
                "CREATE TRIGGER music_metadata_audio_{name} AFTER {event} ON music_metadata_audio
                 BEGIN
                    DELETE FROM music_metadata_discs
                    WHERE id = OLD.disc_id
                        AND NOT EXISTS (SELECT 1 FROM music_metadata_audio WHERE disc_id = OLD.disc_id);
                 END;"
            )).await?;
        }
        // Cleanup can remove the last contributor to an edition.
        manager.get_connection().execute_unprepared(
            "CREATE TRIGGER music_metadata_queue_audio_delete BEFORE DELETE ON music_metadata_audio
             BEGIN
                INSERT OR IGNORE INTO music_metadata_dirty_directories
                    SELECT a.directory_scope FROM music_metadata_discs d
                    JOIN music_metadata_albums a ON a.id = d.album_id WHERE d.id = OLD.disc_id;
             END;",
        ).await?;

        // Queue both previous and current identifiers so edits and deletions revoke evidence.
        for (index, table) in ["music_metadata_audio", "music_metadata_recording_sources"]
            .into_iter()
            .enumerate()
        {
            let (version_columns, version_change) = if index == 0 {
                (
                    ", recording_content_hash, recording_content_size",
                    " OR OLD.recording_content_hash IS NOT NEW.recording_content_hash
                   OR OLD.recording_content_size IS NOT NEW.recording_content_size",
                )
            } else {
                ("", "")
            };
            for (event, rows) in [
                ("INSERT", &["NEW"][..]),
                (
                    "UPDATE OF recording_mbid, recording_isrc",
                    &["OLD", "NEW"][..],
                ),
                ("DELETE", &["OLD"][..]),
            ] {
                let name = event.split_whitespace().next().unwrap_or_default();
                let changed = if name == "UPDATE" {
                    format!(
                        "WHEN OLD.recording_mbid IS NOT NEW.recording_mbid OR OLD.recording_isrc IS NOT NEW.recording_isrc{version_change}"
                    )
                } else {
                    String::new()
                };
                let event = if name == "UPDATE" {
                    format!("{event}{version_columns}")
                } else {
                    event.to_owned()
                };
                let mut body = String::new();
                for row in rows {
                    for (kind, column) in [
                        ("mbid", "recording_mbid"),
                        ("isrc", "recording_isrc"),
                        ("recording", "recording_id"),
                    ] {
                        body.push_str(&format!(
                            "INSERT INTO music_metadata_recording_dirty
                             SELECT '{kind}', {row}.{column} WHERE {row}.{column} IS NOT NULL ON CONFLICT DO NOTHING;"
                        ));
                    }
                }
                manager
                    .get_connection()
                    .execute_unprepared(&format!(
                    "CREATE TRIGGER music_metadata_recording_{index}_{name} AFTER {event} ON {table}
                     {changed} BEGIN {body} END;"
                ))
                    .await?;
            }
        }
        // Replicas can receive an edit before its edition or artist has arrived.
        for event in ["INSERT", "UPDATE", "DELETE"] {
            let row = if event == "DELETE" { "OLD" } else { "NEW" };
            let project_targets = if event == "DELETE" {
                ""
            } else {
                "INSERT INTO music_metadata_album_groups (id, name)
                    SELECT NEW.target_uuid, NEW.label WHERE NEW.kind = 'album' AND NEW.target_uuid IS NOT NULL
                    ON CONFLICT(id) DO NOTHING;
                 INSERT INTO music_metadata_artists (id, name)
                    SELECT NEW.target_uuid, NEW.label WHERE NEW.kind = 'artist'
                    ON CONFLICT(id) DO NOTHING;"
            };
            manager.get_connection().execute_unprepared(&format!(
                "CREATE TRIGGER music_metadata_edit_{event} AFTER {event} ON music_metadata_sync_edits
                 BEGIN
                    {project_targets}
                    UPDATE music_metadata_albums SET group_id = (
                        SELECT target_uuid FROM music_metadata_synced_album_groups groups
                        WHERE groups.album_id = music_metadata_albums.id ORDER BY generation DESC, owner_uuid LIMIT 1)
                    WHERE {row}.kind IN ('album', 'anchor') AND id IN (
                        SELECT album_id FROM music_metadata_edition_matches WHERE edition_uuid = {row}.owner_uuid
                        UNION SELECT a.album_id FROM music_metadata_album_track_anchors a
                        WHERE a.track_uuid = {row}.edition_uuid OR a.track_uuid IN (
                            SELECT edition_uuid FROM music_metadata_sync_edits
                            WHERE kind = 'anchor' AND owner_uuid = {row}.owner_uuid));
                 END;"
            )).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop dependent evidence tables before their owners.
        manager
            .get_connection()
            .execute_unprepared(
                "DROP TRIGGER music_metadata_queue_audio_delete;
             DROP TRIGGER music_metadata_edit_INSERT;
             DROP TRIGGER music_metadata_edit_UPDATE;
             DROP TRIGGER music_metadata_edit_DELETE;
             DROP TRIGGER music_metadata_browse_album;
             DROP TRIGGER music_metadata_browse_artist;
             DROP VIEW music_metadata_browse_artists;
             DROP VIEW music_metadata_browse_albums;
             DROP VIEW music_metadata_current_album_titles;
             DROP VIEW music_metadata_album_effective_fields;
             DROP TABLE music_metadata_browse_handles;
             DROP TRIGGER music_metadata_artist_insert;
             DROP TRIGGER music_metadata_artist_update;
             DROP TABLE music_metadata_details;
             DROP VIEW music_metadata_artist_identities;
             DROP VIEW music_metadata_artist_redirects;
             DROP VIEW music_metadata_synced_album_groups;
             DROP VIEW music_metadata_edition_matches;
             DROP TABLE music_metadata_sync_edits;
             DROP TABLE music_metadata_credits;
             DROP TABLE music_metadata_recording_credits;
             DROP TABLE music_metadata_artists;
             DROP TABLE music_metadata_values;
             DROP TABLE music_metadata_track_release_keys;
             DROP TABLE music_metadata_release_keys;
             DROP TABLE music_metadata_dirty_directories;
             DROP TABLE music_metadata_disc_evidence;
             DROP VIEW music_metadata_live_summaries;
             DROP TABLE music_metadata_album_evidence;
             DROP VIEW music_metadata_track_recordings;
             DROP VIEW music_metadata_recording_evidence;
             DROP VIEW music_metadata_current_recording_audio;
             DROP VIEW music_metadata_recording_aliases;
             DROP TRIGGER music_metadata_recording_content_update;
             DROP TRIGGER music_metadata_disc_delete;
             DROP TABLE music_metadata_local_overlays;
             DROP TABLE music_metadata_track_assets;
             DROP TABLE music_metadata_audio_layouts;
             DROP TABLE music_metadata_release_tracks;
             DROP TABLE music_metadata_audio;
             DROP TABLE music_metadata_dirty_tracks;
             DROP TABLE music_metadata_album_track_anchors;
             DROP TABLE music_metadata_recording_sources;
             DROP TABLE music_metadata_recording_dirty;
             DROP TABLE music_metadata_recordings;
             DROP TABLE music_metadata_discs;
             DROP TABLE music_metadata_dirty_summaries;
             DROP TABLE music_metadata_album_summaries;
             DROP TABLE music_metadata_release_identity;
             DROP TABLE music_metadata_album_field_values;
             DROP TABLE music_metadata_albums;
             DROP TABLE music_metadata_album_groups;
             DROP INDEX music_metadata_file_digest;",
            )
            .await?;
        Ok(())
    }
}
