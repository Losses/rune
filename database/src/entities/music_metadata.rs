//! SeaORM models for the resolved metadata catalog and its read-only projections.

macro_rules! catalog_entity {
    ($module:ident, $table:literal, {$( $(#[$attr:meta])* $field:ident: $ty:ty, )+}) => {
        pub(crate) mod $module {
            use sea_orm::entity::prelude::*;

            #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
            #[sea_orm(table_name = $table)]
            pub struct Model {
                $($(#[$attr])* pub $field: $ty,)+
            }

            #[derive(Clone, Copy, Debug, EnumIter, DeriveRelation)]
            pub enum Relation {}

            impl ActiveModelBehavior for ActiveModel {}
        }
    };
}

catalog_entity!(album_groups, "music_metadata_album_groups", {
    #[sea_orm(primary_key, auto_increment = false)]
    id: String,
    name: String,
});

catalog_entity!(albums, "music_metadata_albums", {
    #[sea_orm(primary_key, auto_increment = false)]
    id: String,
    directory_path: String,
    managed_key: Option<String>,
    directory_scope: String,
    group_id: Option<String>,
    needs_review: i32,
});

catalog_entity!(album_field_values, "music_metadata_album_field_values", {
    #[sea_orm(primary_key, auto_increment = false)]
    album_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    field: String,
    #[sea_orm(primary_key, auto_increment = false)]
    provider: String,
    #[sea_orm(primary_key, auto_increment = false)]
    resource: String,
    value: Option<String>,
    decision: i32,
});

catalog_entity!(release_identity, "music_metadata_release_identity", {
    #[sea_orm(primary_key, auto_increment = false)]
    album_id: String,
    musicbrainz_id: String,
    edition_uuid: String,
});

catalog_entity!(album_summaries, "music_metadata_album_summaries", {
    #[sea_orm(primary_key, auto_increment = false)]
    album_id: String,
    title: String,
    version: Option<String>,
    catalog_number: Option<String>,
    release_date: Option<String>,
    release_type: Option<String>,
    track_count: i64,
});

catalog_entity!(dirty_summaries, "music_metadata_dirty_summaries", {
    #[sea_orm(primary_key, auto_increment = false)]
    album_id: String,
});

catalog_entity!(discs, "music_metadata_discs", {
    #[sea_orm(primary_key, auto_increment = false)]
    id: String,
    album_id: String,
    disc_number: i32,
});

catalog_entity!(recordings, "music_metadata_recordings", {
    #[sea_orm(primary_key, auto_increment = false)]
    id: String,
    musicbrainz_id: Option<String>,
    isrc: Option<String>,
    legacy_isrc: Option<String>,
});

catalog_entity!(recording_sources, "music_metadata_recording_sources", {
    #[sea_orm(primary_key, auto_increment = false)]
    provider: String,
    #[sea_orm(primary_key, auto_increment = false)]
    resource_id: String,
    recording_id: String,
    fallback_id: String,
    recording_mbid: Option<String>,
    recording_isrc: Option<String>,
    codec: Option<String>,
    is_lossless: Option<i32>,
    sample_rate: Option<i32>,
    bits_per_sample: Option<i32>,
    bitrate: Option<i32>,
    available: i32,
});

catalog_entity!(audio, "music_metadata_audio", {
    #[sea_orm(primary_key, auto_increment = false)]
    file_id: i32,
    portable: i32,
    id: String,
    disc_id: String,
    recording_id: String,
    recording_mbid: Option<String>,
    recording_isrc: Option<String>,
    recording_content_hash: Option<String>,
    recording_content_size: Option<i64>,
    release_track_mbid: Option<String>,
    confirmed_release_id: Option<String>,
    title: String,
    track_title_key: String,
    track_number: Option<i32>,
    track_total: Option<i32>,
    disc_number: Option<i32>,
    disc_total: Option<i32>,
    duration_ms: Option<i64>,
    import_date: String,
    subtitle: Option<String>,
    bpm: Option<f64>,
    movement_number: Option<i32>,
    movement_total: Option<i32>,
    file_path: Option<String>,
    file_size_bytes: Option<i64>,
    file_modified_at: Option<String>,
    format: Option<String>,
    codec: Option<String>,
    sample_rate: Option<i32>,
    bits_per_sample: Option<i32>,
    channels: Option<i32>,
    channel_layout: Option<String>,
    bitrate: Option<i32>,
    frame_count: Option<i64>,
    encoder: Option<String>,
    encoder_settings: Option<String>,
    encoding_date: Option<String>,
    tagging_date: Option<String>,
    replaygain_track_gain_db: Option<f64>,
    replaygain_track_peak: Option<f64>,
    replaygain_reference_lufs: Option<f64>,
    lyrics: Option<String>,
    synced_lyrics: Option<String>,
    original_album_title: Option<String>,
    isrc: Option<String>,
    content_type: Option<String>,
    release_date: Option<String>,
    original_release_date: Option<String>,
    version: Option<String>,
    sort_title: Option<String>,
    grouping: Option<String>,
    description: Option<String>,
    comment: Option<String>,
    copyright: Option<String>,
    license: Option<String>,
    script: Option<String>,
    work: Option<String>,
    part: Option<String>,
    section: Option<String>,
    opus: Option<String>,
    work_id: Option<String>,
    recording_date: Option<String>,
    recording_start_date: Option<String>,
    recording_end_date: Option<String>,
    recording_location: Option<String>,
    is_live: Option<i32>,
    is_bootleg: Option<i32>,
});

catalog_entity!(recording_dirty, "music_metadata_recording_dirty", {
    #[sea_orm(primary_key, auto_increment = false)]
    kind: String,
    #[sea_orm(primary_key, auto_increment = false)]
    value: String,
});

catalog_entity!(album_track_anchors, "music_metadata_album_track_anchors", {
    #[sea_orm(primary_key, auto_increment = false)]
    track_uuid: String,
    album_id: String,
});

catalog_entity!(release_tracks, "music_metadata_release_tracks", {
    #[sea_orm(primary_key, auto_increment = false)]
    id: String,
    disc_id: String,
    recording_id: String,
    track_number: Option<i32>,
    title: String,
});

catalog_entity!(dirty_tracks, "music_metadata_dirty_tracks", {
    #[sea_orm(primary_key, auto_increment = false)]
    disc_id: String,
});

catalog_entity!(audio_layouts, "music_metadata_audio_layouts", {
    #[sea_orm(primary_key, auto_increment = false)]
    file_id: i32,
    kind: String,
});

catalog_entity!(track_assets, "music_metadata_track_assets", {
    #[sea_orm(primary_key, auto_increment = false)]
    track_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    file_id: i32,
    #[sea_orm(primary_key, auto_increment = false)]
    start_ms: i64,
    end_ms: Option<i64>,
});

catalog_entity!(local_overlays, "music_metadata_local_overlays", {
    #[sea_orm(primary_key)]
    id: i32,
    album_id: Option<String>,
    disc_id: Option<String>,
    audio_id: Option<String>,
    track_id: Option<String>,
    field: String,
    value: Option<String>,
});

catalog_entity!(artists, "music_metadata_artists", {
    #[sea_orm(primary_key, auto_increment = false, column_name = "source_uuid")]
    id: String,
    name: String,
    group: String,
    musicbrainz_id: Option<String>,
    portable: i32,
});

catalog_entity!(album_evidence, "music_metadata_album_evidence", {
    #[sea_orm(primary_key, auto_increment = false)]
    file_id: i32,
    title: String,
    tag_title: Option<String>,
    title_priority: i32,
    version_priority: i32,
    barcode: Option<String>,
    catalog_number: Option<String>,
    release_country: Option<String>,
    release_type: Option<String>,
    release_status: Option<String>,
    disc_count: Option<i32>,
    declared_track_count: Option<i32>,
    replaygain_album_gain_db: Option<f64>,
    replaygain_album_peak: Option<f64>,
    replaygain_reference_lufs: Option<f64>,
    release_date: Option<String>,
    original_release_date: Option<String>,
    version: Option<String>,
    sort_title: Option<String>,
    grouping: Option<String>,
    description: Option<String>,
    comment: Option<String>,
    copyright: Option<String>,
    license: Option<String>,
    script: Option<String>,
    work: Option<String>,
    part: Option<String>,
    section: Option<String>,
    opus: Option<String>,
    work_id: Option<String>,
    recording_date: Option<String>,
    recording_start_date: Option<String>,
    recording_end_date: Option<String>,
    recording_location: Option<String>,
    is_live: Option<i32>,
    is_bootleg: Option<i32>,
});

catalog_entity!(disc_evidence, "music_metadata_disc_evidence", {
    #[sea_orm(primary_key, auto_increment = false)]
    file_id: i32,
    title: Option<String>,
    title_priority: i32,
    media_format: Option<String>,
    directory_path: Option<String>,
    declared_track_count: Option<i32>,
    release_date: Option<String>,
    original_release_date: Option<String>,
    version: Option<String>,
    sort_title: Option<String>,
    grouping: Option<String>,
    description: Option<String>,
    comment: Option<String>,
    copyright: Option<String>,
    license: Option<String>,
    script: Option<String>,
    work: Option<String>,
    part: Option<String>,
    section: Option<String>,
    opus: Option<String>,
    work_id: Option<String>,
    recording_date: Option<String>,
    recording_start_date: Option<String>,
    recording_end_date: Option<String>,
    recording_location: Option<String>,
    is_live: Option<i32>,
    is_bootleg: Option<i32>,
});

catalog_entity!(track_release_keys, "music_metadata_track_release_keys", {
    #[sea_orm(primary_key, auto_increment = false)]
    file_id: i32,
    album_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    field: String,
    value: String,
});

catalog_entity!(release_keys, "music_metadata_release_keys", {
    #[sea_orm(primary_key, auto_increment = false)]
    album_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    field: String,
    #[sea_orm(primary_key, auto_increment = false)]
    value: String,
    uses: i64,
});

catalog_entity!(dirty_directories, "music_metadata_dirty_directories", {
    #[sea_orm(primary_key, auto_increment = false)]
    directory_scope: String,
});

catalog_entity!(credits, "music_metadata_credits", {
    #[sea_orm(primary_key, auto_increment = false)]
    file_id: i32,
    #[sea_orm(primary_key, auto_increment = false)]
    scope: i32,
    #[sea_orm(primary_key, auto_increment = false)]
    position: i32,
    credited_as: String,
    role: String,
    artist_id: Option<String>,
    artist_confirmed: i32,
    instrument: Option<String>,
    #[sea_orm(primary_key, auto_increment = false)]
    source_kind: String,
    #[sea_orm(primary_key, auto_increment = false)]
    source_provider: String,
    source_key: Option<String>,
    source_path: Option<String>,
    source_pattern: Option<String>,
    source_entity_id: Option<String>,
    source_confidence: Option<f64>,
});

catalog_entity!(recording_credits, "music_metadata_recording_credits", {
    #[sea_orm(primary_key, auto_increment = false)]
    recording_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    position: i32,
    #[sea_orm(primary_key, auto_increment = false)]
    source_kind: String,
    #[sea_orm(primary_key, auto_increment = false)]
    source_provider: String,
    credited_as: String,
    role: String,
    artist_id: Option<String>,
    instrument: Option<String>,
    source_key: Option<String>,
    source_path: Option<String>,
    source_pattern: Option<String>,
    source_entity_id: Option<String>,
    source_confidence: Option<f64>,
});

catalog_entity!(browse_handles, "music_metadata_browse_handles", {
    #[sea_orm(primary_key)]
    id: i32,
    kind: String,
    source_uuid: String,
});

catalog_entity!(values, "music_metadata_values", {
    #[sea_orm(primary_key, auto_increment = false)]
    file_id: i32,
    #[sea_orm(primary_key, auto_increment = false)]
    scope: i32,
    #[sea_orm(primary_key, auto_increment = false)]
    field: String,
    #[sea_orm(primary_key, auto_increment = false)]
    qualifier: String,
    #[sea_orm(primary_key, auto_increment = false)]
    position: i32,
    value: String,
});

catalog_entity!(details, "music_metadata_details", {
    #[sea_orm(primary_key, auto_increment = false)]
    file_id: i32,
    #[sea_orm(primary_key, auto_increment = false)]
    scope: i32,
    artwork: Option<String>,
    links: Option<String>,
    provenance: Option<String>,
});

// Read-only catalog projections. Model only the fields consumed by their callers.
catalog_entity!(current_recording_audio, "music_metadata_current_recording_audio", {
    #[sea_orm(primary_key, auto_increment = false)]
    file_id: i32,
    id: String,
    disc_id: String,
    recording_id: String,
    duration_ms: Option<i64>,
    codec: Option<String>,
    sample_rate: Option<i32>,
    bits_per_sample: Option<i32>,
    bitrate: Option<i64>,
});

catalog_entity!(live_summaries, "music_metadata_live_summaries", {
    #[sea_orm(primary_key, auto_increment = false)]
    album_id: String,
    title: String,
    version: Option<String>,
    catalog_number: Option<String>,
    release_date: Option<String>,
    release_type: Option<String>,
    track_count: i64,
});

catalog_entity!(album_effective_fields, "music_metadata_album_effective_fields", {
    #[sea_orm(primary_key, auto_increment = false)]
    album_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    field: String,
    #[sea_orm(primary_key, auto_increment = false)]
    provider: String,
    #[sea_orm(primary_key, auto_increment = false)]
    resource: String,
    value: Option<String>,
    decision: i32,
});

catalog_entity!(current_album_titles, "music_metadata_current_album_titles", {
    #[sea_orm(primary_key, auto_increment = false)]
    id: String,
    title: String,
    version: Option<String>,
    catalog_number: Option<String>,
    release_date: Option<String>,
    release_type: Option<String>,
    track_count: i64,
});

catalog_entity!(recording_evidence, "music_metadata_recording_evidence", {
    file_id: Option<i32>,
    audio_id: Option<String>,
    provider: Option<String>,
    resource_id: Option<String>,
    #[sea_orm(primary_key, auto_increment = false)]
    recording_id: String,
    mbid: Option<String>,
    isrc: Option<String>,
});

catalog_entity!(recording_aliases, "music_metadata_recording_aliases", {
    #[sea_orm(primary_key, auto_increment = false)]
    source_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    target_id: String,
});

catalog_entity!(track_recordings, "music_metadata_track_recordings", {
    #[sea_orm(primary_key, auto_increment = false)]
    track_id: String,
    recording_id: String,
});

catalog_entity!(edition_matches, "music_metadata_edition_matches", {
    #[sea_orm(primary_key, auto_increment = false)]
    album_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    edition_uuid: String,
});

catalog_entity!(synced_album_groups, "music_metadata_synced_album_groups", {
    #[sea_orm(primary_key, auto_increment = false)]
    album_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    owner_uuid: String,
    target_uuid: Option<String>,
    generation: i64,
});

catalog_entity!(artist_redirects, "music_metadata_artist_redirects", {
    #[sea_orm(primary_key, auto_increment = false)]
    source_uuid: String,
    target_uuid: String,
});

catalog_entity!(artist_identities, "music_metadata_artist_identities", {
    #[sea_orm(primary_key, auto_increment = false)]
    source_id: i32,
    source_uuid: String,
    target_id: i32,
    target_uuid: String,
});

catalog_entity!(browse_albums, "music_metadata_browse_albums", {
    #[sea_orm(primary_key, auto_increment = false)]
    id: i32,
    name: String,
    source_uuid: String,
});

catalog_entity!(browse_artists, "music_metadata_browse_artists", {
    #[sea_orm(primary_key, auto_increment = false)]
    id: i32,
    name: String,
    source_uuid: String,
});
