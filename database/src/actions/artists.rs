use std::collections::HashMap;

use crate::connection::MainDbConnection;
use crate::entities::artists;
use anyhow::Result;

use super::browse::resolved_browse_query;
use super::utils::CollectionDefinition;

pub(super) async fn canonicalize_search_ids(db: &MainDbConnection, ids: &mut [i64]) -> Result<()> {
    use sea_orm::{ConnectionTrait, DbBackend, Statement};

    for ids in ids.chunks_mut(500) {
        let placeholders = vec!["?"; ids.len()].join(", ");
        let rows = db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                format!(
                    "SELECT source_id, target_id FROM music_metadata_artist_identities
                     WHERE source_id IN ({placeholders})"
                ),
                ids.iter().copied().map(Into::into),
            ))
            .await?;
        let mut targets = HashMap::new();
        for row in rows {
            targets.insert(
                row.try_get::<i64>("", "source_id")?,
                row.try_get::<i64>("", "target_id")?,
            );
        }
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

    fn visible() -> sea_orm::Condition {
        sea_orm::Condition::all().add(sea_orm::sea_query::Expr::cust(
            "NOT EXISTS (SELECT 1 FROM music_metadata_artist_redirects WHERE source_uuid = artists.source_uuid)",
        ))
    }
}

resolved_browse_query!(artists::Model, Artist);
