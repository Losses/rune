use crate::entities::portable_albums;

use super::browse::resolved_browse_query;
use super::utils::CollectionDefinition;

impl CollectionDefinition for portable_albums::Entity {
    fn group_column() -> Self::Column {
        portable_albums::Column::Group
    }

    fn id_column() -> Self::Column {
        portable_albums::Column::Id
    }
}

resolved_browse_query!(portable_albums::Model, Album);
