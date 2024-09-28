use std::sync::Arc;

use anyhow::Result;

use database::{
    actions::cover_art::bake_cover_art_by_media_files,
    connection::{MainDbConnection, RecommendationDbConnection},
};

use crate::{query_cover_arts, Collection};

pub async fn inject_cover_art_map(
    main_db: Arc<MainDbConnection>,
    recommend_db: Arc<RecommendationDbConnection>,
    collection: Collection,
) -> Result<Collection> {
    let main_db_for_cover_art_map = Arc::clone(&main_db);
    let files = query_cover_arts(main_db, recommend_db, collection.queries.clone()).await?;
    let cover_art_map = bake_cover_art_by_media_files(&main_db_for_cover_art_map, files).await?;

    Ok(Collection {
        id: collection.id,
        name: collection.name,
        queries: collection.queries,
        collection_type: collection.collection_type,
        cover_art_map,
    })
}
