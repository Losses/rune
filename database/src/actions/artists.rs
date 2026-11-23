use std::collections::HashMap;

use crate::connection::MainDbConnection;
use crate::entities::{
    artists,
    music_metadata::{artist_identities, artist_redirects},
};
use anyhow::Result;
use sea_orm::{
    ColumnTrait, Condition, EntityTrait, ExprTrait, QueryFilter, QuerySelect,
    sea_query::{Expr, Query},
};

use super::browse::resolved_browse_query;
use super::utils::CollectionDefinition;

pub(super) async fn canonicalize_search_ids(db: &MainDbConnection, ids: &mut [i64]) -> Result<()> {
    use artist_identities::Column as Identity;
    for ids in ids.chunks_mut(500) {
        let rows = artist_identities::Entity::find()
            .select_only()
            .columns([Identity::SourceId, Identity::TargetId])
            .filter(Identity::SourceId.is_in(ids.iter().copied()))
            .into_tuple::<(i64, i64)>()
            .all(db)
            .await?;
        let targets: HashMap<_, _> = rows.into_iter().collect();
        for id in ids {
            if let Some(target) = targets.get(id) {
                *id = *target;
            }
        }
    }
    Ok(())
}

impl CollectionDefinition for artists::Entity {
    fn group_column() -> Self::Column {
        artists::Column::Group
    }

    fn id_column() -> Self::Column {
        artists::Column::Id
    }

    fn visible() -> Condition {
        let redirects = Query::select()
            .column(artist_redirects::Column::SourceUuid)
            .from(artist_redirects::Entity)
            .and_where(
                Expr::col(artist_redirects::Column::SourceUuid.as_column_ref())
                    .eq(Expr::col(artists::Column::SourceUuid.as_column_ref())),
            )
            .to_owned();
        Condition::all().add(Expr::not_exists(redirects))
    }
}

resolved_browse_query!(artists::Model, Artist);
