use anyhow::{Context, Result};
use log::error;
use uuid::Uuid;

use sync::{
    core::PrimaryKeyFromStr,
    hlc::{HLCModel, HLCRecord, HLC},
    impl_hlc_model_for_entity, impl_hlc_record_for_model, impl_primary_key_from_str_for_i32_pk,
};

use crate::entities::albums;
use crate::entities::artists;
use crate::entities::genres;
use crate::entities::media_cover_art;
use crate::entities::media_file_albums;
use crate::entities::media_file_artists;
use crate::entities::media_file_genres;
use crate::entities::media_files;

// Albums
impl_hlc_record_for_model!(albums::Model);
impl_hlc_model_for_entity!(
    albums::Entity,
    albums::Column::HlcUuid,
    albums::Column::UpdatedAtHlcTs,
    albums::Column::UpdatedAtHlcVer,
    albums::Column::UpdatedAtHlcNid
);

// Artists
impl_hlc_record_for_model!(artists::Model);
impl_hlc_model_for_entity!(
    artists::Entity,
    artists::Column::HlcUuid,
    artists::Column::UpdatedAtHlcTs,
    artists::Column::UpdatedAtHlcVer,
    artists::Column::UpdatedAtHlcNid
);

// Genres
impl_hlc_record_for_model!(genres::Model);
impl_hlc_model_for_entity!(
    genres::Entity,
    genres::Column::HlcUuid,
    genres::Column::UpdatedAtHlcTs,
    genres::Column::UpdatedAtHlcVer,
    genres::Column::UpdatedAtHlcNid
);

// MediaFiles
impl_hlc_record_for_model!(media_files::Model);
impl_hlc_model_for_entity!(
    media_files::Entity,
    media_files::Column::HlcUuid,
    media_files::Column::UpdatedAtHlcTs,
    media_files::Column::UpdatedAtHlcVer,
    media_files::Column::UpdatedAtHlcNid
);

// MediaFileAlbums
impl_hlc_record_for_model!(media_file_albums::Model);
impl_hlc_model_for_entity!(
    media_file_albums::Entity,
    media_file_albums::Column::HlcUuid,
    media_file_albums::Column::UpdatedAtHlcTs,
    media_file_albums::Column::UpdatedAtHlcVer,
    media_file_albums::Column::UpdatedAtHlcNid
);

// MediaFileArtists
impl_hlc_record_for_model!(media_file_artists::Model);
impl_hlc_model_for_entity!(
    media_file_artists::Entity,
    media_file_artists::Column::HlcUuid,
    media_file_artists::Column::UpdatedAtHlcTs,
    media_file_artists::Column::UpdatedAtHlcVer,
    media_file_artists::Column::UpdatedAtHlcNid
);

// MediaFileGenres
impl_hlc_record_for_model!(media_file_genres::Model);
impl_hlc_model_for_entity!(
    media_file_genres::Entity,
    media_file_genres::Column::HlcUuid,
    media_file_genres::Column::UpdatedAtHlcTs,
    media_file_genres::Column::UpdatedAtHlcVer,
    media_file_genres::Column::UpdatedAtHlcNid
);

// MediaCoverArt
impl_hlc_record_for_model!(media_cover_art::Model);
impl_hlc_model_for_entity!(
    media_cover_art::Entity,
    media_cover_art::Column::HlcUuid,
    media_cover_art::Column::UpdatedAtHlcTs,
    media_cover_art::Column::UpdatedAtHlcVer,
    media_cover_art::Column::UpdatedAtHlcNid
);

impl_primary_key_from_str_for_i32_pk!(albums::PrimaryKey, i32);
impl_primary_key_from_str_for_i32_pk!(artists::PrimaryKey, i32);
impl_primary_key_from_str_for_i32_pk!(genres::PrimaryKey, i32);
impl_primary_key_from_str_for_i32_pk!(media_files::PrimaryKey, i32);
impl_primary_key_from_str_for_i32_pk!(media_file_albums::PrimaryKey, i32);
impl_primary_key_from_str_for_i32_pk!(media_file_artists::PrimaryKey, i32);
impl_primary_key_from_str_for_i32_pk!(media_file_genres::PrimaryKey, i32);
impl_primary_key_from_str_for_i32_pk!(media_cover_art::PrimaryKey, i32);
