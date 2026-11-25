use crate::entities::portable_artists;

use super::browse::resolved_browse_query;
use super::utils::CollectionDefinition;

impl CollectionDefinition for portable_artists::Entity {
    fn group_column() -> Self::Column {
        portable_artists::Column::Group
    }

    fn id_column() -> Self::Column {
        portable_artists::Column::Id
    }
}

resolved_browse_query!(portable_artists::Model, Artist);
