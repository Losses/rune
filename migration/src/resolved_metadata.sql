CREATE TABLE music_metadata_album_groups (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL
) WITHOUT ROWID;

CREATE TABLE music_metadata_albums (
    id TEXT PRIMARY KEY NOT NULL,
    directory_path TEXT NOT NULL,
    managed_key TEXT,
    directory_scope TEXT GENERATED ALWAYS AS (COALESCE('managed:' || managed_key, directory_path)) VIRTUAL,
    group_id TEXT REFERENCES music_metadata_album_groups(id) ON DELETE SET NULL,
    needs_review INTEGER NOT NULL DEFAULT 0 CHECK (needs_review IN (0, 1))
);
CREATE INDEX music_metadata_album_directory ON music_metadata_albums(directory_scope, id);
CREATE INDEX music_metadata_album_group ON music_metadata_albums(group_id) WHERE group_id IS NOT NULL;
CREATE INDEX music_metadata_file_digest ON media_files(content_hash, content_size)
    WHERE content_hash IS NOT NULL;

-- Provider proposals are owned by the edition and survive rescanning its files.
CREATE TABLE music_metadata_album_field_values (
    album_id TEXT NOT NULL REFERENCES music_metadata_albums(id) ON DELETE CASCADE,
    field TEXT NOT NULL CHECK (field IN ('title', 'version', 'catalog_number', 'release_date',
        'original_release_date', 'labels', 'release_type', 'barcode', 'release_country', 'description')),
    provider TEXT NOT NULL,
    resource TEXT NOT NULL,
    value TEXT,
    decision INTEGER NOT NULL CHECK (decision IN (0, 1, 2)),
    PRIMARY KEY (album_id, field, provider, resource),
    CHECK (field <> 'title' OR value IS NOT NULL),
    CHECK (provider <> '' AND resource <> '' AND value IS NOT NULL)
) WITHOUT ROWID;
CREATE UNIQUE INDEX music_metadata_album_field_approved ON music_metadata_album_field_values(album_id, field)
    WHERE decision = 1 AND provider <> '';
CREATE INDEX music_metadata_album_field_lookup ON music_metadata_album_field_values(field, value COLLATE NOCASE, album_id)
    WHERE decision = 1;
CREATE TABLE music_metadata_release_identity (
    album_id TEXT PRIMARY KEY NOT NULL REFERENCES music_metadata_albums(id) ON DELETE CASCADE,
    musicbrainz_id TEXT NOT NULL,
    edition_uuid TEXT NOT NULL
) WITHOUT ROWID;
CREATE INDEX music_metadata_release_identity_mbid ON music_metadata_release_identity(musicbrainz_id);
CREATE INDEX music_metadata_release_identity_edition ON music_metadata_release_identity(edition_uuid);

-- The browse projection is refreshed with the completed edition assignment.
CREATE TABLE music_metadata_album_summaries (
    album_id TEXT PRIMARY KEY NOT NULL REFERENCES music_metadata_albums(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    version TEXT,
    catalog_number TEXT,
    release_date TEXT,
    release_type TEXT CHECK (release_type IN ('album', 'ep', 'single', 'broadcast', 'other')),
    track_count INTEGER NOT NULL CHECK (track_count >= 0)
) WITHOUT ROWID;
CREATE INDEX music_metadata_album_summary_title
    ON music_metadata_album_summaries(title COLLATE NOCASE, album_id);
CREATE INDEX music_metadata_album_summary_type ON music_metadata_album_summaries(release_type, album_id)
    WHERE release_type IS NOT NULL;
CREATE TABLE music_metadata_dirty_summaries (
    album_id TEXT PRIMARY KEY NOT NULL REFERENCES music_metadata_albums(id) ON DELETE CASCADE
) WITHOUT ROWID;

CREATE TABLE music_metadata_discs (
    id TEXT PRIMARY KEY NOT NULL,
    album_id TEXT NOT NULL REFERENCES music_metadata_albums(id),
    disc_number INTEGER NOT NULL CHECK (disc_number > 0),
    UNIQUE (album_id, disc_number)
);

-- A recording can be shared by tracks from several editions and providers.
CREATE TABLE music_metadata_recordings (
    id TEXT PRIMARY KEY NOT NULL,
    musicbrainz_id TEXT,
    isrc TEXT,
    legacy_isrc TEXT UNIQUE
) WITHOUT ROWID;
CREATE INDEX music_metadata_recording_mbid ON music_metadata_recordings(musicbrainz_id)
    WHERE musicbrainz_id IS NOT NULL;
CREATE INDEX music_metadata_recording_isrc ON music_metadata_recordings(isrc) WHERE isrc IS NOT NULL;
CREATE TABLE music_metadata_recording_sources (
    provider TEXT NOT NULL,
    resource_id TEXT NOT NULL,
    recording_id TEXT NOT NULL REFERENCES music_metadata_recordings(id),
    recording_mbid TEXT,
    recording_isrc TEXT,
    codec TEXT, is_lossless INTEGER, sample_rate INTEGER, bits_per_sample INTEGER, bitrate INTEGER,
    available INTEGER NOT NULL DEFAULT 1 CHECK (available IN (0, 1)),
    PRIMARY KEY (provider, resource_id)
) WITHOUT ROWID;
CREATE INDEX music_metadata_recording_sources_recording ON music_metadata_recording_sources(recording_id);

-- Track properties and parent membership are directly queryable.
CREATE TABLE music_metadata_audio (
    file_id INTEGER PRIMARY KEY REFERENCES media_files(id) ON DELETE CASCADE,
    id TEXT NOT NULL UNIQUE,
    disc_id TEXT NOT NULL REFERENCES music_metadata_discs(id),
    recording_id TEXT NOT NULL REFERENCES music_metadata_recordings(id),
    recording_mbid TEXT,
    recording_isrc TEXT,
    recording_content_hash TEXT,
    recording_content_size INTEGER,
    release_track_mbid TEXT,
    confirmed_release_id TEXT REFERENCES music_metadata_albums(id) ON DELETE SET NULL,
    title TEXT NOT NULL,
    track_title_key TEXT NOT NULL,
    track_number INTEGER, track_total INTEGER, disc_number INTEGER, disc_total INTEGER,
    duration_ms INTEGER, import_date TEXT NOT NULL,
    subtitle TEXT, bpm REAL, movement_number INTEGER, movement_total INTEGER,
    file_path TEXT, file_size_bytes INTEGER, file_modified_at TEXT,
    format TEXT, codec TEXT, sample_rate INTEGER, bits_per_sample INTEGER,
    channels INTEGER, channel_layout TEXT, bitrate INTEGER, frame_count INTEGER,
    encoder TEXT, encoder_settings TEXT, encoding_date TEXT, tagging_date TEXT,
    replaygain_track_gain_db REAL, replaygain_track_peak REAL, replaygain_reference_lufs REAL,
    lyrics TEXT, synced_lyrics TEXT, original_album_title TEXT, isrc TEXT, content_type TEXT,
    release_date TEXT, original_release_date TEXT, version TEXT, sort_title TEXT,
    grouping TEXT, description TEXT, comment TEXT, copyright TEXT, license TEXT, script TEXT,
    work TEXT, part TEXT, section TEXT, opus TEXT, work_id TEXT, recording_date TEXT,
    recording_start_date TEXT, recording_end_date TEXT, recording_location TEXT,
    is_live INTEGER, is_bootleg INTEGER
);
CREATE INDEX music_metadata_audio_disc_tracks
    ON music_metadata_audio(disc_id, (track_number IS NULL), track_number, file_id);
CREATE INDEX music_metadata_audio_confirmed_release
    ON music_metadata_audio(confirmed_release_id) WHERE confirmed_release_id IS NOT NULL;
CREATE INDEX music_metadata_audio_recording ON music_metadata_audio(recording_id);
CREATE INDEX music_metadata_audio_release_track ON music_metadata_audio(release_track_mbid)
    WHERE release_track_mbid IS NOT NULL;

-- Keep identifier assertions on the resource that supplied them.
CREATE INDEX music_metadata_audio_recording_mbid ON music_metadata_audio(recording_mbid) WHERE recording_mbid IS NOT NULL;
CREATE INDEX music_metadata_audio_recording_isrc ON music_metadata_audio(recording_isrc) WHERE recording_isrc IS NOT NULL;
CREATE INDEX music_metadata_source_recording_mbid ON music_metadata_recording_sources(recording_mbid)
    WHERE recording_mbid IS NOT NULL;
CREATE INDEX music_metadata_source_recording_isrc ON music_metadata_recording_sources(recording_isrc)
    WHERE recording_isrc IS NOT NULL;
CREATE TABLE music_metadata_recording_dirty (
    kind TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (kind, value)
) WITHOUT ROWID;
CREATE VIEW music_metadata_current_recording_audio AS
SELECT a.* FROM music_metadata_audio a JOIN media_files f ON f.id = a.file_id
WHERE a.recording_content_hash IS f.content_hash AND a.recording_content_size IS f.content_size;

CREATE VIEW music_metadata_recording_evidence AS
    SELECT file_id, id AS audio_id, NULL AS provider, NULL AS resource_id, recording_id,
           recording_mbid AS mbid, recording_isrc AS isrc FROM music_metadata_current_recording_audio
    UNION ALL
    SELECT NULL, NULL, provider, resource_id, recording_id, recording_mbid, recording_isrc
    FROM music_metadata_recording_sources;

-- An ISRC-only UUID can follow an unambiguous upgrade without moving its credited history.
CREATE VIEW music_metadata_recording_aliases AS
SELECT old.id AS source_id, current.id AS target_id
FROM music_metadata_recordings old JOIN music_metadata_recordings current ON current.isrc = old.legacy_isrc
WHERE old.isrc IS NULL AND current.id <> old.id AND NOT EXISTS (
    SELECT 1 FROM music_metadata_recordings conflicting WHERE conflicting.isrc = current.isrc
    AND conflicting.id <> current.id)
UNION ALL
SELECT t.recording_id, r.recording_id FROM music_metadata_release_tracks t
JOIN music_metadata_track_recordings r ON r.track_id = t.id WHERE r.recording_id <> t.recording_id;

CREATE TRIGGER music_metadata_recording_content_update AFTER UPDATE OF content_hash, content_size ON media_files
WHEN OLD.content_hash IS NOT NEW.content_hash OR OLD.content_size IS NOT NEW.content_size
BEGIN
    INSERT OR IGNORE INTO music_metadata_recording_dirty
    SELECT 'mbid', recording_mbid FROM music_metadata_audio WHERE file_id = NEW.id AND recording_mbid IS NOT NULL;
    INSERT OR IGNORE INTO music_metadata_recording_dirty
    SELECT 'isrc', recording_isrc FROM music_metadata_audio WHERE file_id = NEW.id AND recording_isrc IS NOT NULL;
    INSERT OR IGNORE INTO music_metadata_recording_dirty
    SELECT 'recording', recording_id FROM music_metadata_audio WHERE file_id = NEW.id;
END;

-- Shared track UUIDs resolve edition selections on devices with their own catalog UUIDs.
CREATE TABLE music_metadata_album_track_anchors (
    track_uuid TEXT PRIMARY KEY NOT NULL,
    album_id TEXT NOT NULL REFERENCES music_metadata_albums(id) ON DELETE CASCADE
) WITHOUT ROWID;
CREATE INDEX music_metadata_album_track_anchors_album ON music_metadata_album_track_anchors(album_id, track_uuid);

-- A position on a release can link to multiple encodings or a range in a disc image.
CREATE TABLE music_metadata_release_tracks (
    id TEXT PRIMARY KEY NOT NULL,
    disc_id TEXT NOT NULL REFERENCES music_metadata_discs(id) ON DELETE CASCADE,
    recording_id TEXT NOT NULL REFERENCES music_metadata_recordings(id),
    track_number INTEGER CHECK (track_number >= 0),
    title TEXT NOT NULL
) WITHOUT ROWID;
CREATE INDEX music_metadata_release_tracks_disc ON music_metadata_release_tracks(disc_id, track_number, id);
CREATE INDEX music_metadata_release_tracks_recording ON music_metadata_release_tracks(recording_id);
CREATE TABLE music_metadata_dirty_tracks (
    disc_id TEXT PRIMARY KEY NOT NULL REFERENCES music_metadata_discs(id) ON DELETE CASCADE
) WITHOUT ROWID;
CREATE TRIGGER music_metadata_tracks_audio_insert AFTER INSERT ON music_metadata_audio
BEGIN
    INSERT INTO music_metadata_dirty_tracks VALUES (NEW.disc_id) ON CONFLICT DO NOTHING;
END;
CREATE TRIGGER music_metadata_tracks_audio_update AFTER UPDATE OF disc_id, track_number, track_title_key, title, release_track_mbid
ON music_metadata_audio
WHEN OLD.disc_id <> NEW.disc_id OR OLD.track_number IS NOT NEW.track_number OR OLD.title <> NEW.title
    OR OLD.track_title_key <> NEW.track_title_key OR OLD.release_track_mbid IS NOT NEW.release_track_mbid
BEGIN
    INSERT INTO music_metadata_dirty_tracks VALUES (OLD.disc_id), (NEW.disc_id) ON CONFLICT DO NOTHING;
END;
CREATE TRIGGER music_metadata_tracks_audio_delete BEFORE DELETE ON music_metadata_audio
BEGIN
    INSERT INTO music_metadata_dirty_tracks VALUES (OLD.disc_id) ON CONFLICT DO NOTHING;
END;
CREATE TABLE music_metadata_audio_layouts (
    file_id INTEGER PRIMARY KEY REFERENCES music_metadata_audio(file_id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('manual', 'cue'))
);
CREATE TABLE music_metadata_track_assets (
    track_id TEXT NOT NULL REFERENCES music_metadata_release_tracks(id) ON DELETE CASCADE,
    file_id INTEGER NOT NULL REFERENCES music_metadata_audio(file_id) ON DELETE CASCADE,
    start_ms INTEGER NOT NULL CHECK (start_ms >= 0),
    end_ms INTEGER CHECK (end_ms > start_ms),
    PRIMARY KEY (track_id, file_id, start_ms)
) WITHOUT ROWID;
CREATE INDEX music_metadata_track_assets_file ON music_metadata_track_assets(file_id, start_ms, track_id);

-- A logical track supplies its own recording identity until verified whole-file evidence agrees.
CREATE VIEW music_metadata_track_recordings AS
SELECT t.id AS track_id, COALESCE(
    (SELECT min(a.recording_id) FROM music_metadata_track_assets v
     JOIN music_metadata_current_recording_audio a ON a.file_id = v.file_id
     JOIN music_metadata_recordings r ON r.id = a.recording_id
     WHERE v.track_id = t.id AND v.start_ms = 0 AND v.end_ms IS NULL
       AND (r.musicbrainz_id IS NOT NULL OR r.isrc IS NOT NULL)
     HAVING count(DISTINCT a.recording_id) = 1), t.recording_id) AS recording_id
FROM music_metadata_release_tracks t;

-- A sparse local source can mask a field without modifying lower-priority evidence.
CREATE TABLE music_metadata_local_overlays (
    id INTEGER PRIMARY KEY,
    album_id TEXT REFERENCES music_metadata_albums(id) ON DELETE CASCADE,
    disc_id TEXT REFERENCES music_metadata_discs(id) ON DELETE CASCADE,
    audio_id TEXT REFERENCES music_metadata_audio(id) ON DELETE CASCADE,
    track_id TEXT REFERENCES music_metadata_release_tracks(id) ON DELETE CASCADE,
    field TEXT NOT NULL CHECK (field IN ('title','version','catalog_number','release_date',
        'original_release_date','labels','release_type','barcode','release_country','description',
        'comment','sort_title','subtitle')),
    value TEXT,
    CHECK ((album_id IS NOT NULL) + (disc_id IS NOT NULL) + (audio_id IS NOT NULL)
           + (track_id IS NOT NULL) = 1),
    CHECK (field <> 'title' OR value IS NOT NULL OR disc_id IS NOT NULL),
    UNIQUE(album_id, field), UNIQUE(disc_id, field), UNIQUE(audio_id, field), UNIQUE(track_id, field)
);
CREATE INDEX music_metadata_local_overlay_lookup ON music_metadata_local_overlays(field, value COLLATE NOCASE, album_id);
CREATE VIEW music_metadata_album_effective_fields AS
SELECT album_id, field, '' AS provider, '' AS resource, value, 1 AS decision
FROM music_metadata_local_overlays WHERE album_id IS NOT NULL
UNION ALL
SELECT f.* FROM music_metadata_album_field_values f WHERE f.decision = 1 AND NOT EXISTS (
    SELECT 1 FROM music_metadata_local_overlays o WHERE o.album_id = f.album_id AND o.field = f.field);

CREATE TRIGGER music_metadata_track_asset_deleted AFTER DELETE ON music_metadata_track_assets
BEGIN
    DELETE FROM music_metadata_release_tracks WHERE id = OLD.track_id AND NOT EXISTS (
        SELECT 1 FROM music_metadata_track_assets WHERE track_id = OLD.track_id);
END;
CREATE TRIGGER music_metadata_summary_track_insert AFTER INSERT ON music_metadata_release_tracks
BEGIN
    INSERT OR IGNORE INTO music_metadata_dirty_summaries
    SELECT album_id FROM music_metadata_discs WHERE id = NEW.disc_id;
END;
CREATE TRIGGER music_metadata_summary_track_delete BEFORE DELETE ON music_metadata_release_tracks
BEGIN
    INSERT OR IGNORE INTO music_metadata_dirty_summaries
    SELECT album_id FROM music_metadata_discs WHERE id = OLD.disc_id;
END;
CREATE TRIGGER music_metadata_summary_track_move BEFORE UPDATE OF disc_id ON music_metadata_release_tracks
WHEN OLD.disc_id <> NEW.disc_id
BEGIN
    INSERT OR IGNORE INTO music_metadata_dirty_summaries
    SELECT album_id FROM music_metadata_discs WHERE id IN (OLD.disc_id, NEW.disc_id);
END;

-- Known identities are independent of the legacy name index used by older library clients.
CREATE TABLE music_metadata_artists (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    musicbrainz_id TEXT UNIQUE
) WITHOUT ROWID;
CREATE INDEX music_metadata_artist_name ON music_metadata_artists(name COLLATE NOCASE, id);
INSERT INTO music_metadata_artists (id, name) SELECT source_uuid, name FROM artists;
CREATE TRIGGER music_metadata_artist_insert AFTER INSERT ON artists
BEGIN
    INSERT INTO music_metadata_artists (id, name) VALUES (NEW.source_uuid, NEW.name)
    ON CONFLICT(id) DO UPDATE SET name = excluded.name;
END;
CREATE TRIGGER music_metadata_artist_update AFTER UPDATE OF name ON artists
BEGIN
    UPDATE music_metadata_artists SET name = NEW.name WHERE id = NEW.source_uuid;
END;

-- Keep each track's scalar evidence so removals and rescans can resolve the next candidate.
CREATE TABLE music_metadata_album_evidence (
    file_id INTEGER PRIMARY KEY REFERENCES music_metadata_audio(file_id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    title_priority INTEGER NOT NULL,
    version_priority INTEGER NOT NULL,
    barcode TEXT, catalog_number TEXT, release_country TEXT,
    release_type TEXT CHECK (release_type IN ('album', 'ep', 'single', 'broadcast', 'other')),
    release_status TEXT,
    disc_count INTEGER, declared_track_count INTEGER,
    replaygain_album_gain_db REAL, replaygain_album_peak REAL, replaygain_reference_lufs REAL,
    release_date TEXT, original_release_date TEXT, version TEXT, sort_title TEXT,
    grouping TEXT, description TEXT, comment TEXT, copyright TEXT, license TEXT, script TEXT,
    work TEXT, part TEXT, section TEXT, opus TEXT, work_id TEXT, recording_date TEXT,
    recording_start_date TEXT, recording_end_date TEXT, recording_location TEXT,
    is_live INTEGER, is_bootleg INTEGER
);
CREATE TABLE music_metadata_disc_evidence (
    file_id INTEGER PRIMARY KEY REFERENCES music_metadata_audio(file_id) ON DELETE CASCADE,
    title TEXT, title_priority INTEGER NOT NULL,
    media_format TEXT, directory_path TEXT, declared_track_count INTEGER,
    release_date TEXT, original_release_date TEXT, version TEXT, sort_title TEXT,
    grouping TEXT, description TEXT, comment TEXT, copyright TEXT, license TEXT, script TEXT,
    work TEXT, part TEXT, section TEXT, opus TEXT, work_id TEXT, recording_date TEXT,
    recording_start_date TEXT, recording_end_date TEXT, recording_location TEXT,
    is_live INTEGER, is_bootleg INTEGER
);

-- Invalidation follows the evidence even during an interrupted scan or a synced deletion.
CREATE TRIGGER music_metadata_summary_evidence_insert AFTER INSERT ON music_metadata_album_evidence
BEGIN
    INSERT OR IGNORE INTO music_metadata_dirty_summaries
    SELECT d.album_id FROM music_metadata_audio a JOIN music_metadata_discs d ON d.id = a.disc_id
    WHERE a.file_id = NEW.file_id;
END;
CREATE TRIGGER music_metadata_summary_evidence_update AFTER UPDATE ON music_metadata_album_evidence
BEGIN
    INSERT OR IGNORE INTO music_metadata_dirty_summaries
    SELECT d.album_id FROM music_metadata_audio a JOIN music_metadata_discs d ON d.id = a.disc_id
    WHERE a.file_id = NEW.file_id;
END;
CREATE TRIGGER music_metadata_summary_evidence_delete AFTER DELETE ON music_metadata_album_evidence
BEGIN
    INSERT OR IGNORE INTO music_metadata_dirty_summaries
    SELECT d.album_id FROM music_metadata_audio a JOIN music_metadata_discs d ON d.id = a.disc_id
    WHERE a.file_id = OLD.file_id;
END;
CREATE TRIGGER music_metadata_summary_audio_move BEFORE UPDATE OF disc_id, track_number ON music_metadata_audio
WHEN OLD.disc_id <> NEW.disc_id OR OLD.track_number IS NOT NEW.track_number
BEGIN
    INSERT OR IGNORE INTO music_metadata_dirty_summaries
    SELECT album_id FROM music_metadata_discs WHERE id IN (OLD.disc_id, NEW.disc_id);
END;
CREATE TRIGGER music_metadata_summary_audio_delete BEFORE DELETE ON music_metadata_audio
BEGIN
    INSERT OR IGNORE INTO music_metadata_dirty_summaries
    SELECT album_id FROM music_metadata_discs WHERE id = OLD.disc_id;
END;

-- Only dirty editions need to resolve fields from track rows on the read path.
CREATE VIEW music_metadata_live_summaries AS
WITH ordered AS (
    SELECT d.album_id,
        FIRST_VALUE(e.title) OVER (PARTITION BY d.album_id ORDER BY e.title_priority,
            d.disc_number, a.track_number IS NULL, a.track_number, a.file_id) AS title,
        FIRST_VALUE(e.version) OVER (PARTITION BY d.album_id ORDER BY e.version IS NULL,
            e.version_priority, d.disc_number, a.track_number IS NULL, a.track_number, a.file_id) AS version,
        FIRST_VALUE(e.catalog_number) OVER (PARTITION BY d.album_id ORDER BY e.catalog_number IS NULL,
            d.disc_number, a.track_number IS NULL, a.track_number, a.file_id) AS catalog_number,
        FIRST_VALUE(e.release_date) OVER (PARTITION BY d.album_id ORDER BY e.release_date IS NULL,
            d.disc_number, a.track_number IS NULL, a.track_number, a.file_id) AS release_date,
        FIRST_VALUE(e.release_type) OVER (PARTITION BY d.album_id ORDER BY e.release_type IS NULL,
            d.disc_number, a.track_number IS NULL, a.track_number, a.file_id) AS release_type,
        ROW_NUMBER() OVER (PARTITION BY d.album_id ORDER BY a.file_id) AS pick
    FROM music_metadata_dirty_summaries dirty
    CROSS JOIN music_metadata_discs d ON d.album_id = dirty.album_id
    JOIN music_metadata_audio a ON a.disc_id = d.id
    JOIN music_metadata_album_evidence e ON e.file_id = a.file_id
)
SELECT album_id, title, version, catalog_number, release_date, release_type,
    (SELECT count(*) FROM music_metadata_release_tracks t JOIN music_metadata_discs td ON td.id = t.disc_id
     WHERE td.album_id = ordered.album_id) AS track_count
FROM ordered WHERE pick = 1;

-- Track contributions maintain the distinct release keys used for subsequent matching.
CREATE TABLE music_metadata_track_release_keys (
    file_id INTEGER NOT NULL REFERENCES music_metadata_audio(file_id) ON DELETE CASCADE,
    album_id TEXT NOT NULL REFERENCES music_metadata_albums(id) ON DELETE CASCADE,
    field TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (file_id, field)
) WITHOUT ROWID;
CREATE INDEX music_metadata_track_release_keys_album ON music_metadata_track_release_keys(album_id);
CREATE TABLE music_metadata_release_keys (
    album_id TEXT NOT NULL REFERENCES music_metadata_albums(id) ON DELETE CASCADE,
    field TEXT NOT NULL,
    value TEXT NOT NULL,
    uses INTEGER NOT NULL CHECK (uses > 0),
    PRIMARY KEY (album_id, field, value)
) WITHOUT ROWID;

-- A committed scan can be reconciled later even if its caller is interrupted.
CREATE TABLE music_metadata_dirty_directories (
    directory_scope TEXT PRIMARY KEY NOT NULL
) WITHOUT ROWID;

-- Keep matching independent of the number of tracks already indexed in a release.
CREATE TRIGGER music_metadata_release_key_insert AFTER INSERT ON music_metadata_track_release_keys
BEGIN
    INSERT INTO music_metadata_release_keys (album_id, field, value, uses)
    VALUES (NEW.album_id, NEW.field, NEW.value, 1)
    ON CONFLICT (album_id, field, value) DO UPDATE SET uses = uses + 1;
END;
CREATE TRIGGER music_metadata_release_key_delete AFTER DELETE ON music_metadata_track_release_keys
BEGIN
    DELETE FROM music_metadata_release_keys
    WHERE album_id = OLD.album_id AND field = OLD.field AND value = OLD.value AND uses = 1;
    UPDATE music_metadata_release_keys SET uses = uses - 1
    WHERE album_id = OLD.album_id AND field = OLD.field AND value = OLD.value;
END;

-- Artist associations are optional so a scan can keep a credit before disambiguation.
CREATE TABLE music_metadata_credits (
    file_id INTEGER NOT NULL REFERENCES music_metadata_audio(file_id) ON DELETE CASCADE,
    scope INTEGER NOT NULL CHECK (scope BETWEEN 0 AND 2),
    position INTEGER NOT NULL CHECK (position >= 0),
    credited_as TEXT NOT NULL,
    role TEXT NOT NULL,
    artist_id TEXT REFERENCES music_metadata_artists(id) ON DELETE SET NULL,
    artist_confirmed INTEGER NOT NULL DEFAULT 0 CHECK (artist_confirmed IN (0, 1)),
    instrument TEXT,
    source_kind TEXT NOT NULL,
    source_provider TEXT NOT NULL,
    source_key TEXT, source_path TEXT, source_pattern TEXT, source_entity_id TEXT,
    source_confidence REAL,
    PRIMARY KEY (file_id, source_kind, source_provider, scope, position)
) WITHOUT ROWID;
CREATE INDEX music_metadata_credits_lookup
    ON music_metadata_credits(scope, role, credited_as COLLATE NOCASE, file_id);
CREATE INDEX music_metadata_credits_artist ON music_metadata_credits(artist_id)
    WHERE artist_id IS NOT NULL;

-- Recording-level credits can exist before a particular file or edition is present.
CREATE TABLE music_metadata_recording_credits (
    recording_id TEXT NOT NULL REFERENCES music_metadata_recordings(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    source_kind TEXT NOT NULL,
    source_provider TEXT NOT NULL,
    credited_as TEXT NOT NULL,
    role TEXT NOT NULL,
    artist_id TEXT REFERENCES music_metadata_artists(id) ON DELETE SET NULL,
    instrument TEXT,
    source_key TEXT, source_path TEXT, source_pattern TEXT, source_entity_id TEXT,
    source_confidence REAL,
    PRIMARY KEY (recording_id, source_kind, source_provider, position)
) WITHOUT ROWID;
CREATE INDEX music_metadata_recording_credits_artist ON music_metadata_recording_credits(artist_id)
    WHERE artist_id IS NOT NULL;

-- Manual decisions are replicated separately from local parser evidence.
CREATE TABLE music_metadata_sync_edits (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_uuid TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK (kind IN ('album', 'artist', 'anchor')),
    owner_uuid TEXT NOT NULL,
    edition_uuid TEXT,
    target_uuid TEXT,
    label TEXT,
    generation INTEGER NOT NULL DEFAULT 0 CHECK (generation >= 0)
);
CREATE INDEX music_metadata_sync_edit_owner ON music_metadata_sync_edits(kind, owner_uuid, edition_uuid);
CREATE INDEX music_metadata_sync_edit_edition ON music_metadata_sync_edits(edition_uuid) WHERE edition_uuid IS NOT NULL;
CREATE INDEX music_metadata_sync_edit_target ON music_metadata_sync_edits(kind, target_uuid);

-- Shared tracks locate an edition on a peer. They are never its edit key.
CREATE VIEW music_metadata_edition_matches AS
SELECT id AS album_id, id AS edition_uuid FROM music_metadata_albums
UNION
SELECT min(album_id), edition_uuid FROM music_metadata_release_identity
GROUP BY edition_uuid HAVING count(*) = 1
UNION
SELECT min(a.album_id), e.owner_uuid FROM music_metadata_sync_edits e
JOIN music_metadata_album_track_anchors a ON a.track_uuid = e.edition_uuid
WHERE e.kind = 'anchor' AND NOT EXISTS (
    SELECT 1 FROM music_metadata_albums local WHERE local.id = e.owner_uuid)
    AND (SELECT count(*) FROM music_metadata_release_identity known
         WHERE known.edition_uuid = e.owner_uuid) <> 1
GROUP BY e.owner_uuid HAVING count(DISTINCT a.album_id) = 1;

CREATE VIEW music_metadata_synced_album_groups AS
SELECT m.album_id, e.owner_uuid, e.target_uuid, e.generation
FROM music_metadata_sync_edits e JOIN music_metadata_edition_matches m ON m.edition_uuid = e.owner_uuid
WHERE e.kind = 'album';

-- Resolve transitive merges and converge on one representative if concurrent merges form a cycle.
CREATE VIEW music_metadata_artist_redirects AS
WITH RECURSIVE reachable(source_uuid, target_uuid) AS (
    SELECT owner_uuid, target_uuid FROM music_metadata_sync_edits WHERE kind = 'artist' AND target_uuid IS NOT NULL
    UNION
    SELECT r.source_uuid, next.target_uuid FROM reachable r JOIN music_metadata_sync_edits next
        ON next.kind = 'artist' AND next.owner_uuid = r.target_uuid WHERE next.target_uuid IS NOT NULL
), canonical AS (
    SELECT r.source_uuid, COALESCE(
        MIN(CASE WHEN NOT EXISTS (SELECT 1 FROM music_metadata_sync_edits outgoing
            WHERE outgoing.kind = 'artist' AND outgoing.owner_uuid = r.target_uuid) THEN r.target_uuid END),
        MIN(CASE WHEN EXISTS (SELECT 1 FROM reachable loop
            WHERE loop.source_uuid = r.target_uuid AND loop.target_uuid = r.target_uuid) THEN r.target_uuid END)
    ) AS target_uuid FROM reachable r GROUP BY r.source_uuid
)
SELECT source_uuid, target_uuid FROM canonical WHERE source_uuid <> target_uuid;
CREATE VIEW music_metadata_artist_identities AS
    SELECT a.id AS source_id, a.source_uuid, COALESCE(t.id, a.id) AS target_id,
           COALESCE(r.target_uuid, a.source_uuid) AS target_uuid
    FROM artists a LEFT JOIN music_metadata_artist_redirects r ON r.source_uuid = a.source_uuid
    LEFT JOIN artists t ON t.source_uuid = r.target_uuid;

-- Reserve negative browse IDs for richer identities while positive IDs stay compatible.
CREATE TABLE music_metadata_browse_handles (
    id INTEGER PRIMARY KEY AUTOINCREMENT CHECK (id <= 2147483647),
    kind TEXT NOT NULL CHECK (kind IN ('album','artist')),
    source_uuid TEXT NOT NULL,
    UNIQUE(kind, source_uuid)
);
CREATE TRIGGER music_metadata_browse_album AFTER INSERT ON music_metadata_albums
BEGIN
    INSERT INTO music_metadata_browse_handles (kind, source_uuid) VALUES ('album', NEW.id)
    ON CONFLICT(kind, source_uuid) DO NOTHING;
END;
CREATE TRIGGER music_metadata_browse_artist AFTER INSERT ON music_metadata_artists
BEGIN
    INSERT INTO music_metadata_browse_handles (kind, source_uuid) VALUES ('artist', NEW.id)
    ON CONFLICT(kind, source_uuid) DO NOTHING;
END;
INSERT INTO music_metadata_browse_handles (kind, source_uuid)
SELECT 'artist', id FROM music_metadata_artists;

CREATE VIEW music_metadata_current_album_titles AS
SELECT a.id, COALESCE(title.value, live.title, s.title, a.directory_path) AS title,
       CASE WHEN version.album_id IS NOT NULL THEN version.value
            WHEN live.album_id IS NOT NULL THEN live.version ELSE s.version END AS version,
       CASE WHEN catalog.album_id IS NOT NULL THEN catalog.value
            WHEN live.album_id IS NOT NULL THEN live.catalog_number ELSE s.catalog_number END AS catalog_number,
       CASE WHEN date.album_id IS NOT NULL THEN date.value
            WHEN live.album_id IS NOT NULL THEN live.release_date ELSE s.release_date END AS release_date,
       CASE WHEN kind.album_id IS NOT NULL THEN kind.value
            WHEN live.album_id IS NOT NULL THEN live.release_type ELSE s.release_type END AS release_type,
       COALESCE(live.track_count, CASE WHEN dirty.album_id IS NULL THEN s.track_count END, 0) AS track_count
FROM music_metadata_albums a
LEFT JOIN music_metadata_album_summaries s ON s.album_id = a.id
LEFT JOIN music_metadata_dirty_summaries dirty ON dirty.album_id = a.id
LEFT JOIN music_metadata_live_summaries live ON live.album_id = dirty.album_id
LEFT JOIN music_metadata_album_effective_fields title ON title.album_id = a.id AND title.field = 'title'
LEFT JOIN music_metadata_album_effective_fields version ON version.album_id = a.id AND version.field = 'version'
LEFT JOIN music_metadata_album_effective_fields catalog ON catalog.album_id = a.id AND catalog.field = 'catalog_number'
LEFT JOIN music_metadata_album_effective_fields date ON date.album_id = a.id AND date.field = 'release_date'
LEFT JOIN music_metadata_album_effective_fields kind ON kind.album_id = a.id AND kind.field = 'release_type';
CREATE VIEW music_metadata_browse_albums AS
SELECT -h.id AS id, a.title AS name, a.id AS source_uuid
FROM music_metadata_current_album_titles a
JOIN music_metadata_browse_handles h ON h.kind = 'album' AND h.source_uuid = a.id
WHERE a.track_count > 0
UNION ALL
SELECT a.id, a.name, a.source_uuid FROM albums a
WHERE EXISTS (
    SELECT 1 FROM media_file_albums l WHERE l.album_uuid = a.source_uuid
    AND NOT EXISTS (SELECT 1 FROM music_metadata_audio track WHERE track.id = l.track_uuid))
    OR NOT EXISTS (SELECT 1 FROM media_file_albums l WHERE l.album_uuid = a.source_uuid);
CREATE VIEW music_metadata_browse_artists AS
SELECT COALESCE(old.id, -h.id) AS id, a.name, a.id AS source_uuid
FROM music_metadata_artists a
JOIN music_metadata_browse_handles h ON h.kind = 'artist' AND h.source_uuid = a.id
LEFT JOIN artists old ON old.source_uuid = a.id
WHERE NOT EXISTS (SELECT 1 FROM music_metadata_artist_redirects r WHERE r.source_uuid = a.id)
AND (a.musicbrainz_id IS NOT NULL OR (
    old.id IS NOT NULL AND (NOT EXISTS (
        SELECT 1 FROM music_metadata_artists known WHERE known.musicbrainz_id IS NOT NULL
        AND known.name = a.name COLLATE NOCASE)
    OR EXISTS (
        SELECT 1 FROM media_file_artists l LEFT JOIN music_metadata_audio track ON track.id = l.track_uuid
        WHERE l.artist_uuid = a.id AND NOT EXISTS (
            SELECT 1 FROM music_metadata_credits c WHERE c.file_id = track.file_id AND c.scope = 2
            AND c.role = 'Primary Artist' AND c.artist_id IS NOT NULL AND c.credited_as = a.name COLLATE NOCASE)))));

-- Identifiers and other repeatable text tags retain their input order and scope.
CREATE TABLE music_metadata_values (
    file_id INTEGER NOT NULL REFERENCES music_metadata_audio(file_id) ON DELETE CASCADE,
    scope INTEGER NOT NULL CHECK (scope BETWEEN 0 AND 2),
    field TEXT NOT NULL,
    qualifier TEXT NOT NULL,
    position INTEGER NOT NULL CHECK (position >= 0),
    value TEXT NOT NULL,
    PRIMARY KEY (file_id, scope, field, qualifier, position)
) WITHOUT ROWID;
CREATE INDEX music_metadata_values_lookup
    ON music_metadata_values(scope, field, qualifier, value COLLATE NOCASE, file_id);

-- Structured details are loaded only in full metadata queries.
CREATE TABLE music_metadata_details (
    file_id INTEGER NOT NULL REFERENCES music_metadata_audio(file_id) ON DELETE CASCADE,
    scope INTEGER NOT NULL CHECK (scope BETWEEN 0 AND 2),
    artwork TEXT,
    links TEXT,
    provenance TEXT,
    PRIMARY KEY (file_id, scope)
) WITHOUT ROWID;

CREATE TRIGGER music_metadata_disc_delete AFTER DELETE ON music_metadata_discs
BEGIN
    DELETE FROM music_metadata_albums WHERE id = OLD.album_id AND group_id IS NULL
        AND NOT EXISTS (SELECT 1 FROM music_metadata_album_field_values WHERE album_id = OLD.album_id)
        AND NOT EXISTS (SELECT 1 FROM music_metadata_local_overlays WHERE album_id = OLD.album_id)
        AND NOT EXISTS (SELECT 1 FROM music_metadata_discs WHERE album_id = OLD.album_id);
END;
