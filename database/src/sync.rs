use sync::hlc::HLCModel;

use crate::entities::{
    albums, artists, genres, media_analysis, media_cover_art, media_file_playlists, media_files,
    media_metadata, mix_queries, mixes, playlists,
};

impl HLCModel for albums::Entity {
    fn updated_at_time_column() -> Self::Column {
        albums::Column::UpdatedAt
    }
    fn updated_at_version_column() -> Self::Column {
        albums::Column::DataVersion
    }
    fn unique_id_column() -> Self::Column {
        albums::Column::Name
    }
}

impl HLCModel for artists::Entity {
    fn updated_at_time_column() -> Self::Column {
        artists::Column::UpdatedAt
    }
    fn updated_at_version_column() -> Self::Column {
        artists::Column::DataVersion
    }
    fn unique_id_column() -> Self::Column {
        artists::Column::Name
    }
}

impl HLCModel for genres::Entity {
    fn updated_at_time_column() -> Self::Column {
        genres::Column::UpdatedAt
    }
    fn updated_at_version_column() -> Self::Column {
        genres::Column::DataVersion
    }
    fn unique_id_column() -> Self::Column {
        genres::Column::Name
    }
}

impl HLCModel for media_analysis::Entity {
    fn updated_at_time_column() -> Self::Column {
        media_analysis::Column::UpdatedAt
    }
    fn updated_at_version_column() -> Self::Column {
        media_analysis::Column::DataVersion
    }
    fn unique_id_column() -> Self::Column {
        media_analysis::Column::FileId
    }
}

impl HLCModel for media_cover_art::Entity {
    fn updated_at_time_column() -> Self::Column {
        media_cover_art::Column::UpdatedAt
    }
    fn updated_at_version_column() -> Self::Column {
        media_cover_art::Column::DataVersion
    }
    fn unique_id_column() -> Self::Column {
        media_cover_art::Column::Id
    }
}

impl HLCModel for media_files::Entity {
    fn updated_at_time_column() -> Self::Column {
        media_files::Column::UpdatedAt
    }
    fn updated_at_version_column() -> Self::Column {
        media_files::Column::DataVersion
    }
    fn unique_id_column() -> Self::Column {
        media_files::Column::Id
    }
}

impl HLCModel for media_metadata::Entity {
    fn updated_at_time_column() -> Self::Column {
        media_metadata::Column::UpdatedAt
    }
    fn updated_at_version_column() -> Self::Column {
        media_metadata::Column::DataVersion
    }
    fn unique_id_column() -> Self::Column {
        media_metadata::Column::FileId
    }
}

impl HLCModel for mixes::Entity {
    fn updated_at_time_column() -> Self::Column {
        mixes::Column::UpdatedAt
    }
    fn updated_at_version_column() -> Self::Column {
        mixes::Column::DataVersion
    }
    fn unique_id_column() -> Self::Column {
        mixes::Column::Id
    }
}

impl HLCModel for mix_queries::Entity {
    fn updated_at_time_column() -> Self::Column {
        mix_queries::Column::UpdatedAt
    }
    fn updated_at_version_column() -> Self::Column {
        mix_queries::Column::DataVersion
    }
    fn unique_id_column() -> Self::Column {
        mix_queries::Column::Id
    }
}

impl HLCModel for playlists::Entity {
    fn updated_at_time_column() -> Self::Column {
        playlists::Column::UpdatedAt
    }
    fn updated_at_version_column() -> Self::Column {
        playlists::Column::DataVersion
    }
    fn unique_id_column() -> Self::Column {
        playlists::Column::Id
    }
}

impl HLCModel for media_file_playlists::Entity {
    fn updated_at_time_column() -> Self::Column {
        media_file_playlists::Column::UpdatedAt
    }
    fn updated_at_version_column() -> Self::Column {
        media_file_playlists::Column::DataVersion
    }
    fn unique_id_column() -> Self::Column {
        media_file_playlists::Column::Id
    }
}
