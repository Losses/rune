use crate::entities::albums;

use super::browse::resolved_browse_query;
use super::utils::CollectionDefinition;

impl CollectionDefinition for albums::Entity {
    fn group_column() -> Self::Column {
        albums::Column::Group
    }

    fn id_column() -> Self::Column {
        albums::Column::Id
    }
}

resolved_browse_query!(albums::Model, Album);
