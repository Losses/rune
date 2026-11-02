use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use log::warn;
use migration::ExprTrait;
use migration::Func;
use migration::IntoCondition;
use migration::SimpleExpr;
use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{Condition, Expr, Query};
use sea_orm::{
    ColumnTrait, EntityTrait, JoinType, Order, QueryFilter, QueryOrder, QuerySelect, QueryTrait,
    TransactionTrait,
};

use crate::actions::analysis::get_analyze_count;
use crate::actions::analysis::get_percentile_analysis_result;
use crate::actions::playback_queue::list_playback_queue;
use crate::connection::{MainDbConnection, RecommendationDbConnection};
use crate::entities::media_file_fingerprint;
use crate::entities::media_file_genres;
use crate::entities::{
    albums, artists, genres, media_analysis, media_file_albums, media_file_artists,
    media_file_likes, media_file_playlists, media_file_stats, media_files, mix_queries, mixes,
    playlists,
};
use crate::sync::{
    SeaOrmSyncStore,
    model::{Collection, ObjectKey},
};
use sea_orm::{DatabaseTransaction, Set};

use super::analysis::get_centralized_analysis_result;
use super::collection::CollectionQuery;
use super::collection::CollectionQueryListMode;
use super::collection::CollectionQueryType;
use super::file::get_files_by_ids;
use super::recommendation::get_recommendation_by_parameter;
use super::utils::CollectionDefinition;

impl CollectionDefinition for mixes::Entity {
    fn group_column() -> Self::Column {
        mixes::Column::Group
    }

    fn id_column() -> Self::Column {
        mixes::Column::Id
    }
}

pub async fn get_mixes_groups(
    db: &DatabaseConnection,
    groups: Vec<String>,
) -> Result<Vec<(String, Vec<(mixes::Model, HashSet<i32>)>)>> {
    let entities: Vec<mixes::Model> = mixes::Entity::find()
        .filter(mixes::Column::Group.is_in(groups.clone()))
        .all(db)
        .await?;

    let mut grouped_entities: HashMap<String, Vec<mixes::Model>> = HashMap::new();
    for entity in entities {
        grouped_entities
            .entry(entity.group.clone())
            .or_default()
            .push(entity);
    }

    let result = groups
        .into_iter()
        .map(|group| {
            let entities_in_group = grouped_entities
                .remove(&group)
                .unwrap_or_default()
                .into_iter()
                .map(|x| (x, HashSet::new()))
                .collect();
            (group, entities_in_group)
        })
        .collect();

    Ok(result)
}

#[async_trait]
impl CollectionQuery for mixes::Model {
    fn collection_type() -> CollectionQueryType {
        CollectionQueryType::Mix
    }

    async fn query_builder(main_db: &MainDbConnection, id: i32) -> Result<Vec<(String, String)>> {
        executable_mix_queries(main_db, id).await
    }

    async fn count_by_first_letter(main_db: &MainDbConnection) -> Result<Vec<(String, i32)>> {
        let results = mixes::Entity::find()
            .select_only()
            .column(mixes::Column::Group)
            .column_as(mixes::Column::Id.count(), "count")
            .group_by(mixes::Column::Group)
            .into_tuple::<(String, i32)>()
            .all(main_db)
            .await
            .with_context(|| "Failed to count collection by first letter")?;

        Ok(results)
    }

    async fn get_groups(
        main_db: &MainDbConnection,
        group_titles: Vec<String>,
    ) -> Result<Vec<(String, Vec<(Self, HashSet<i32>)>)>> {
        get_mixes_groups(main_db, group_titles)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get collection groups: {e}"))
    }

    async fn get_by_ids(main_db: &MainDbConnection, ids: &[i32]) -> Result<Vec<Self>> {
        mixes::Entity::find()
            .filter(mixes::Column::Id.is_in(ids.to_vec()))
            .all(main_db)
            .await
            .with_context(|| "Failed to get collection item by ids")
    }

    async fn list(
        main_db: &MainDbConnection,
        limit: u64,
        mode: CollectionQueryListMode,
    ) -> Result<Vec<Self>> {
        use sea_orm::{
            FromQueryResult, Order, QueryOrder, QuerySelect, QueryTrait, sea_query::Func,
            sea_query::SimpleExpr,
        };

        match mode {
            CollectionQueryListMode::Name => {
                mixes::Entity::find()
                    .order_by_asc(mixes::Column::Name)
                    .limit(limit)
                    .all(main_db)
                    .await
            }
            CollectionQueryListMode::Forward => {
                mixes::Entity::find().limit(limit).all(main_db).await
            }
            CollectionQueryListMode::Reverse => {
                mixes::Entity::find()
                    .order_by_desc(mixes::Column::Id)
                    .limit(limit)
                    .all(main_db)
                    .await
            }

            CollectionQueryListMode::Random => {
                let mut query: sea_orm::sea_query::SelectStatement =
                    mixes::Entity::find().as_query().to_owned();

                let select = query
                    .order_by_expr(SimpleExpr::FunctionCall(Func::random()), Order::Asc)
                    .limit(limit);

                let statement = main_db.get_database_backend().build(select);

                mixes::Model::find_by_statement(statement)
                    .all(main_db)
                    .await
            }
        }
        .with_context(|| "Failed to get collection list")
    }

    fn id(&self) -> i32 {
        self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn readonly(&self) -> bool {
        self.locked
    }
}

async fn author_mix(
    tx: &DatabaseTransaction,
    store: &SeaOrmSyncStore,
    mix: &mixes::Model,
) -> Result<()> {
    crate::sync::model::author_source_on(
        tx,
        store,
        ObjectKey {
            collection: Collection::Mixes,
            id: mix.hlc_uuid.parse()?,
        },
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn create_mix(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    node_id: &str,
    name: String,
    group: String,
    scriptlet_mode: bool,
    mode: i32,
    locked: bool,
    queries: Vec<(String, String)>,
) -> Result<mixes::Model> {
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let now = Utc::now().to_rfc3339();
    let row = mixes::ActiveModel {
        name: Set(name),
        group: Set(group),
        scriptlet_mode: Set(scriptlet_mode),
        mode: Set(Some(mode)),
        locked: Set(locked),
        hlc_uuid: Set(Uuid::new_v4().to_string()),
        created_at_hlc_ts: Set(now.clone()),
        updated_at_hlc_ts: Set(now),
        created_at_hlc_ver: Set(0),
        updated_at_hlc_ver: Set(0),
        created_at_hlc_nid: Set(node_id.to_owned()),
        updated_at_hlc_nid: Set(node_id.to_owned()),
        ..Default::default()
    }
    .insert(&tx)
    .await?;
    crate::actions::search::add_term(&tx, CollectionQueryType::Mix, row.id, &row.name).await?;
    replace_queries_on(&tx, &row.hlc_uuid, queries, 0).await?;
    author_mix(&tx, store, &row).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn get_all_mixes(db: &DatabaseConnection) -> Result<Vec<mixes::Model>> {
    Ok(mixes::Entity::find().all(db).await?)
}
pub async fn get_mix_by_id(db: &DatabaseConnection, id: i32) -> Result<mixes::Model> {
    mixes::Entity::find_by_id(id)
        .one(db)
        .await?
        .context("Mix not found")
}

#[allow(clippy::too_many_arguments)]
pub async fn update_mix(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    node_id: &str,
    id: i32,
    name: Option<String>,
    group: Option<String>,
    scriptlet_mode: Option<bool>,
    mode: Option<i32>,
    locked: Option<bool>,
    queries: Option<Vec<(String, String)>>,
) -> Result<mixes::Model> {
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let row = mixes::Entity::find_by_id(id)
        .one(&tx)
        .await?
        .context("Mix not found")?;
    let ver = row.updated_at_hlc_ver;
    let mut model: mixes::ActiveModel = row.into();
    if let Some(v) = name {
        model.name = Set(v);
    }
    if let Some(v) = group {
        model.group = Set(v);
    }
    if let Some(v) = scriptlet_mode {
        model.scriptlet_mode = Set(v);
    }
    if let Some(v) = mode {
        model.mode = Set(Some(v));
    }
    if let Some(v) = locked {
        model.locked = Set(v);
    }
    model.updated_at_hlc_ts = Set(Utc::now().to_rfc3339());
    model.updated_at_hlc_ver = Set(ver.checked_add(1).context("mix version exhausted")?);
    model.updated_at_hlc_nid = Set(node_id.to_owned());
    let row = model.update(&tx).await?;
    if let Some(queries) = queries {
        replace_queries_on(&tx, &row.hlc_uuid, queries, 0).await?;
    }
    crate::actions::search::add_term(&tx, CollectionQueryType::Mix, row.id, &row.name).await?;
    author_mix(&tx, store, &row).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn remove_mix(db: &DatabaseConnection, store: &SeaOrmSyncStore, id: i32) -> Result<()> {
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let row = mixes::Entity::find_by_id(id)
        .one(&tx)
        .await?
        .context("mix not found")?;
    let key = ObjectKey {
        collection: Collection::Mixes,
        id: row.hlc_uuid.parse()?,
    };
    crate::actions::search::remove_term(&tx, CollectionQueryType::Mix, id).await?;
    crate::sync::bindings::apply(&tx, &key, None).await?;
    store.author_on(&tx, key, None).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn replace_mix_queries(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    mix_id: i32,
    mix_hlc_uuid: &str,
    queries: Vec<(String, String)>,
    group: Option<i32>,
) -> Result<()> {
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let mix = mixes::Entity::find_by_id(mix_id)
        .one(&tx)
        .await?
        .context("mix not found")?;
    anyhow::ensure!(mix.hlc_uuid == mix_hlc_uuid, "mix identity changed");
    replace_queries_on(&tx, &mix.hlc_uuid, queries, group.unwrap_or_default()).await?;
    author_mix(&tx, store, &mix).await?;
    tx.commit().await?;
    Ok(())
}
pub async fn get_mix_queries_by_mix_id(
    db: &DatabaseConnection,
    mix_id: i32,
) -> Result<Vec<MixQueryView>> {
    let mix = mixes::Entity::find_by_id(mix_id)
        .one(db)
        .await?
        .context("mix not found")?;
    query_views_on(db, &mix.hlc_uuid).await
}
pub async fn executable_mix_queries(
    db: &DatabaseConnection,
    mix_id: i32,
) -> Result<Vec<(String, String)>> {
    let mix = mixes::Entity::find_by_id(mix_id)
        .one(db)
        .await?
        .context("mix not found")?;
    executable_queries_on(db, &mix.hlc_uuid).await
}
pub async fn remove_mix_query(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    id: i32,
) -> Result<()> {
    use sync_orm::owned_list::{ListBudget, OwnedListPolicy};
    type Clauses = OwnedListPolicy<mix_queries::Entity, crate::sync::model::MixClauseMap>;
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let row = mix_queries::Entity::find_by_id(id)
        .one(&tx)
        .await?
        .context("mix clause not found")?;
    let mix = mixes::Entity::find()
        .filter(mixes::Column::HlcUuid.eq(&row.mix_uuid))
        .one(&tx)
        .await?
        .context("mix not found")?;
    let owner = [row.mix_uuid.into()];
    let mut items = Clauses::read_on(&tx, &owner, Clauses::read_ceiling()).await?;
    items.retain(|item| item.id.to_string() != row.hlc_uuid);
    for (index, item) in items.iter_mut().enumerate() {
        item.position = index as i32;
    }
    Clauses::prepare_on(
        &tx,
        &owner,
        &items,
        ListBudget {
            max_reads: 129,
            max_mutations: 384,
        },
    )
    .await?
    .apply_on(&tx)
    .await?;
    author_mix(&tx, store, &mix).await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Debug)]
enum QueryOperator {
    LibAll(bool),
    LibArtist(i32),
    LibAlbum(i32),
    LibGenre(i32),
    LibPlaylist(i32),
    LibTrack(i32),
    LibRandom(i32),
    LibQueue(bool),
    LibDirectoryDeep(String),
    LibDirectoryShallow(String),
    SortTrackNumber(bool),
    SortLastModified(bool),
    SortDuration(bool),
    SortPlayedthrough(bool),
    SortSkipped(bool),
    FilterLiked(bool),
    FilterWithCoverArt(bool),
    FilterAnalyzed(bool),
    PipeLimit(u64),
    PipeRecommend(i32),
    Unknown(String),
}

fn parse_parameter<T>(parameter: &str, operator: &str) -> Option<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Debug,
{
    match parameter.parse::<T>() {
        Ok(x) => Some(x),
        Err(_) => {
            warn!("Unable to parse the parameter of operator: {operator}({parameter})");
            None
        }
    }
}

pub async fn add_item_to_mix(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    mix_id: i32,
    operator: String,
    parameter: String,
) -> Result<mix_queries::Model> {
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let mix = mixes::Entity::find_by_id(mix_id)
        .one(&tx)
        .await?
        .context("mix not found")?;
    let row = add_query_on(&tx, &mix.hlc_uuid, operator, parameter).await?;
    author_mix(&tx, store, &mix).await?;
    tx.commit().await?;
    Ok(row)
}

/// Initialize built-in clauses only before a replication namespace exists.
pub async fn initialize_mix_queries_before_replication(db: &DatabaseConnection) -> Result<()> {
    let tx = db.begin().await?;
    anyhow::ensure!(
        sync_orm::causal_store::list_namespace_ids_on(&tx, 1)
            .await?
            .is_empty(),
        "replica already initialized"
    );
    let all = mixes::Entity::find()
        .filter(
            Condition::all()
                .add(mixes::Column::Name.starts_with("\u{200B}"))
                .add(mixes::Column::Group.eq("\u{200B}Rune"))
                .add(mixes::Column::Locked.eq(true))
                .add(mixes::Column::ScriptletMode.eq(false)),
        )
        .all(&tx)
        .await?;
    for mix in all {
        if mix_queries::Entity::find()
            .filter(mix_queries::Column::MixUuid.eq(&mix.hlc_uuid))
            .count(&tx)
            .await?
            == 0
        {
            let mut queries = vec![("lib::all".to_owned(), "true".to_owned())];
            if mix.name == "\u{200B}Liked" {
                queries.push(("filter::liked".into(), "true".into()));
            } else if mix.name.starts_with("\u{200B}Mix ")
                && let Some(n) = mix.name.split_whitespace().last()
            {
                queries.push(("pipe::limit".into(), "50".into()));
                queries.push(("pipe::recommend".into(), n.into()));
            }
            replace_queries_on(&tx, &mix.hlc_uuid, queries, 0).await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

fn parse_query(query: &(String, String)) -> QueryOperator {
    let (operator, parameter) = query;
    match operator.as_str() {
        "lib::all" => parse_parameter::<bool>(parameter, operator)
            .map(QueryOperator::LibAll)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "lib::artist" => parse_parameter::<i32>(parameter, operator)
            .map(QueryOperator::LibArtist)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "lib::album" => parse_parameter::<i32>(parameter, operator)
            .map(QueryOperator::LibAlbum)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "lib::genre" => parse_parameter::<i32>(parameter, operator)
            .map(QueryOperator::LibGenre)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "lib::playlist" => parse_parameter::<i32>(parameter, operator)
            .map(QueryOperator::LibPlaylist)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "lib::track" => parse_parameter::<i32>(parameter, operator)
            .map(QueryOperator::LibTrack)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "lib::random" => parse_parameter::<i32>(parameter, operator)
            .map(QueryOperator::LibRandom)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "lib::queue" => parse_parameter::<bool>(parameter, operator)
            .map(QueryOperator::LibQueue)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "lib::directory.deep" => QueryOperator::LibDirectoryDeep(parameter.clone()),
        "lib::directory.shallow" => QueryOperator::LibDirectoryShallow(parameter.clone()),
        "sort::track_number" => parse_parameter::<bool>(parameter, operator)
            .map(QueryOperator::SortTrackNumber)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "sort::last_modified" => parse_parameter::<bool>(parameter, operator)
            .map(QueryOperator::SortLastModified)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "sort::duration" => parse_parameter::<bool>(parameter, operator)
            .map(QueryOperator::SortDuration)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "sort::playedthrough" => parse_parameter::<bool>(parameter, operator)
            .map(QueryOperator::SortPlayedthrough)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "sort::skipped" => parse_parameter::<bool>(parameter, operator)
            .map(QueryOperator::SortSkipped)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "filter::liked" => parse_parameter::<bool>(parameter, operator)
            .map(QueryOperator::FilterLiked)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "filter::analyzed" => parse_parameter::<bool>(parameter, operator)
            .map(QueryOperator::FilterAnalyzed)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "filter::with_cover_art" => parse_parameter::<bool>(parameter, operator)
            .map(QueryOperator::FilterWithCoverArt)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "pipe::limit" => parse_parameter::<u64>(parameter, operator)
            .map(QueryOperator::PipeLimit)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        "pipe::recommend" => parse_parameter::<i32>(parameter, operator)
            .map(QueryOperator::PipeRecommend)
            .unwrap_or(QueryOperator::Unknown(operator.clone())),
        _ => QueryOperator::Unknown(operator.clone()),
    }
}

fn apply_join_filter(
    query: Select<media_files::Entity>,
    _filter_liked: Option<bool>,
    _filter_analyzed: Option<bool>,
    sort_track_number: Option<bool>,
    _sort_playedthrough_asc: Option<bool>,
    _sort_skipped_asc: Option<bool>,
) -> Select<media_files::Entity> {
    let mut _query = query;

    if sort_track_number.is_some() {
        _query = _query
            .join(
                JoinType::LeftJoin,
                media_file_albums::Relation::MediaFiles.def().rev(),
            )
            .column(media_file_albums::Column::TrackNumber);
    }

    _query
}

// Macro to handle sorting
macro_rules! apply_sorting_macro {
    ($query:expr, $column:expr, $sort_option:expr) => {
        if let Some(asc) = $sort_option {
            $query = $query.order_by($column, if asc { Order::Asc } else { Order::Desc });
        }
    };
}

// Macro to handle subquery filters
macro_rules! add_subquery_filter {
    ($or_condition:expr, $ids:expr, $parent:ident, $entity:ty, $column:expr, $file_column:expr) => {
        if !$ids.is_empty() {
            let parents = $parent::Entity::find()
                .select_only()
                .column($parent::Column::HlcUuid)
                .filter($parent::Column::Id.is_in($ids))
                .into_query();
            let subquery = <$entity>::find()
                .select_only()
                .filter($column.in_subquery(parents))
                .column($file_column)
                .into_query();

            $or_condition = $or_condition.add(
                Expr::col((media_files::Entity, media_files::Column::HlcUuid))
                    .in_subquery(subquery),
            );
        }
    };
}

fn statistics_sum(column: media_file_stats::Column) -> SimpleExpr {
    let sum = Query::select()
        .expr(Func::sum(Expr::col((media_file_stats::Entity, column))))
        .from(media_file_stats::Entity)
        .and_where(
            Expr::col((
                media_file_stats::Entity,
                media_file_stats::Column::TrackUuid,
            ))
            .equals((media_files::Entity, media_files::Column::HlcUuid)),
        )
        .to_owned();
    Func::coalesce([Expr::from(sum), Expr::val(0)]).into()
}

fn sort_media_files(
    mut media_files: Vec<media_files::Model>,
    track_ids: &[i32],
) -> Vec<media_files::Model> {
    let track_id_map: HashMap<i32, usize> = track_ids
        .iter()
        .enumerate()
        .map(|(i, &id)| (id, i))
        .collect();

    media_files.sort_by(|a, b| {
        let a_index = track_id_map.get(&a.id);
        let b_index = track_id_map.get(&b.id);

        match (a_index, b_index) {
            (Some(a_idx), Some(b_idx)) => a_idx.cmp(b_idx),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    });

    media_files
}

pub async fn query_mix_media_files(
    main_db: &DatabaseConnection,
    recommend_db: &RecommendationDbConnection,
    queries: Vec<(String, String)>,
    cursor: usize,
    page_size: usize,
) -> Result<Vec<media_files::Model>> {
    let mut all: bool = false;

    let mut artist_ids: Vec<i32> = vec![];
    let mut album_ids: Vec<i32> = vec![];
    let mut genre_ids: Vec<i32> = vec![];
    let mut playlist_ids: Vec<i32> = vec![];
    let mut track_ids: Vec<i32> = vec![];
    let mut random_count: Vec<i32> = vec![];
    let mut directories_deep: Vec<String> = vec![];
    let mut directories_shallow: Vec<String> = vec![];
    let mut playback_queue: Option<bool> = None;

    let mut sort_track_number_asc: Option<bool> = None;
    let mut sort_last_modified_asc: Option<bool> = None;
    let mut sort_duration_asc: Option<bool> = None;
    let mut sort_playedthrough_asc: Option<bool> = None;
    let mut sort_skipped_asc: Option<bool> = None;

    let mut filter_liked: Option<bool> = None;
    let mut filter_cover_art: Option<bool> = None;
    let mut filter_analyzed: Option<bool> = None;
    let mut pipe_limit: Option<u64> = None;
    let mut pipe_recommend: Option<i32> = None;

    for query in queries {
        match parse_query(&query) {
            QueryOperator::LibAll(is_all) => all = is_all,
            QueryOperator::LibArtist(id) => artist_ids.push(id),
            QueryOperator::LibAlbum(id) => album_ids.push(id),
            QueryOperator::LibGenre(id) => genre_ids.push(id),
            QueryOperator::LibPlaylist(id) => playlist_ids.push(id),
            QueryOperator::LibTrack(id) => track_ids.push(id),
            QueryOperator::LibRandom(count) => random_count.push(count),
            QueryOperator::LibQueue(enabled) => playback_queue = Some(enabled),
            QueryOperator::LibDirectoryDeep(dir) => directories_deep.push(dir),
            QueryOperator::LibDirectoryShallow(dir) => directories_shallow.push(dir),
            QueryOperator::SortTrackNumber(asc) => sort_track_number_asc = Some(asc),
            QueryOperator::SortLastModified(asc) => sort_last_modified_asc = Some(asc),
            QueryOperator::SortDuration(asc) => sort_duration_asc = Some(asc),
            QueryOperator::SortPlayedthrough(asc) => sort_playedthrough_asc = Some(asc),
            QueryOperator::SortSkipped(asc) => sort_skipped_asc = Some(asc),
            QueryOperator::FilterLiked(liked) => filter_liked = Some(liked),
            QueryOperator::FilterWithCoverArt(cover_art) => filter_cover_art = Some(cover_art),
            QueryOperator::FilterAnalyzed(analyzed) => filter_analyzed = Some(analyzed),
            QueryOperator::PipeLimit(limit) => pipe_limit = Some(limit),
            QueryOperator::PipeRecommend(recommend) => pipe_recommend = Some(recommend),
            QueryOperator::Unknown(op) => warn!("Unknown operator: {op}"),
        }
    }

    if pipe_recommend.is_some() && cursor > 0 {
        return Ok([].to_vec());
    }

    if pipe_recommend.is_some() && get_analyze_count(main_db).await? < 1 {
        return Ok([].to_vec());
    }

    if pipe_recommend.is_some() {
        filter_analyzed = Some(true);
    }

    let only_one_playlist = artist_ids.is_empty()
        && album_ids.is_empty()
        && track_ids.is_empty()
        && genre_ids.is_empty()
        && random_count.is_empty()
        && directories_deep.is_empty()
        && directories_shallow.is_empty()
        && playlist_ids.len() == 1;

    let only_playlist = if only_one_playlist {
        playlist_ids[0]
    } else {
        -1
    };

    // Exclude a duplicate only when its fingerprint matches the algorithm,
    // configuration, format, and file content checked below. NOT EXISTS avoids
    // repeating a file for each matching fingerprint row.
    let duplicates = Query::select()
        .expr(Expr::val(1))
        .from(media_file_fingerprint::Entity)
        .and_where(
            Expr::col((
                media_file_fingerprint::Entity,
                media_file_fingerprint::Column::TrackUuid,
            ))
            .equals((media_files::Entity, media_files::Column::HlcUuid)),
        )
        .and_where(
            Expr::col((
                media_file_fingerprint::Entity,
                media_file_fingerprint::Column::IsDuplicated,
            ))
            .eq(1),
        )
        .and_where(
            Expr::col((
                media_file_fingerprint::Entity,
                media_file_fingerprint::Column::Algorithm,
            ))
            .eq("chromaprint"),
        )
        .and_where(
            Expr::col((
                media_file_fingerprint::Entity,
                media_file_fingerprint::Column::Configuration,
            ))
            .eq("rusty-chromaprint-0.3.0/default"),
        )
        .and_where(
            Expr::col((
                media_file_fingerprint::Entity,
                media_file_fingerprint::Column::FormatVersion,
            ))
            .eq(1),
        )
        .and_where(
            Expr::col((
                media_file_fingerprint::Entity,
                media_file_fingerprint::Column::SourceHash,
            ))
            .equals((media_files::Entity, media_files::Column::ContentHash)),
        )
        .and_where(
            Expr::col((
                media_file_fingerprint::Entity,
                media_file_fingerprint::Column::SourceSize,
            ))
            .equals((media_files::Entity, media_files::Column::ContentSize)),
        )
        .to_owned();
    let mut query = media_files::Entity::find().filter(Expr::not_exists(duplicates));

    // Create an OR condition to hold all the subconditions
    let mut or_condition = Condition::any();

    // Filter by artist_ids if provided
    add_subquery_filter!(
        or_condition,
        artist_ids,
        artists,
        media_file_artists::Entity,
        media_file_artists::Column::ArtistUuid,
        media_file_artists::Column::TrackUuid
    );

    // Filter by album_ids if provided
    add_subquery_filter!(
        or_condition,
        album_ids,
        albums,
        media_file_albums::Entity,
        media_file_albums::Column::AlbumUuid,
        media_file_albums::Column::TrackUuid
    );

    // Filter by genres_ids if provided
    add_subquery_filter!(
        or_condition,
        genre_ids,
        genres,
        media_file_genres::Entity,
        media_file_genres::Column::GenreUuid,
        media_file_genres::Column::TrackUuid
    );

    // Filter by playlist_ids if provided
    add_subquery_filter!(
        or_condition,
        playlist_ids.clone(),
        playlists,
        media_file_playlists::Entity,
        media_file_playlists::Column::PlaylistUuid,
        media_file_playlists::Column::TrackUuid
    );

    // Filter by track_ids if provided
    if !track_ids.is_empty() {
        let subquery = media_files::Entity::find()
            .select_only()
            .filter(media_files::Column::Id.is_in(track_ids.clone()))
            .column(media_files::Column::Id)
            .into_query();

        or_condition = or_condition
            .add(Expr::col((media_files::Entity, media_files::Column::Id)).in_subquery(subquery));
    }

    // Filter by directories if provided
    if !directories_deep.is_empty() {
        let mut dir_conditions = Condition::any();
        for dir in directories_deep {
            let dir = dir.strip_prefix('/').unwrap_or(&dir);

            dir_conditions = dir_conditions.add(
                Expr::col(media_files::Column::Directory)
                    .eq(dir)
                    .or(Expr::col(media_files::Column::Directory).like(format!("{dir}/%"))),
            );
        }
        or_condition = or_condition.add(dir_conditions);
    }

    // Filter by directories if provided
    if !directories_shallow.is_empty() {
        let mut dir_conditions = Condition::any();
        for dir in directories_shallow {
            let dir = dir.strip_prefix('/').unwrap_or(&dir);

            dir_conditions = dir_conditions.add(Expr::col(media_files::Column::Directory).eq(dir));
        }
        or_condition = or_condition.add(dir_conditions);
    }

    // Filter by random tracks if provided
    if !random_count.is_empty() {
        let subquery = media_files::Entity::find()
            .select_only()
            .order_by(SimpleExpr::FunctionCall(Func::random()), Order::Asc)
            .limit(*random_count.iter().max().unwrap_or(&30) as u64)
            .column(media_files::Column::Id)
            .into_query();

        or_condition = or_condition
            .add(Expr::col((media_files::Entity, media_files::Column::Id)).in_subquery(subquery));
    }

    if let Some(queue_enabled) = playback_queue
        && queue_enabled
    {
        let queued_tracks = list_playback_queue(main_db).await?;

        let subquery = media_files::Entity::find()
            .select_only()
            .filter(media_files::Column::Id.is_in(queued_tracks))
            .column(media_files::Column::Id)
            .into_query();

        or_condition = or_condition
            .add(Expr::col((media_files::Entity, media_files::Column::Id)).in_subquery(subquery));
    }

    let has_liked = filter_liked.is_some();
    let has_cover_art = filter_cover_art.is_some();
    let has_analyzed = filter_analyzed.is_some();

    if has_liked || has_cover_art || has_analyzed {
        let mut filter = Condition::all();

        if !all {
            filter = filter.add(or_condition);
        }

        if let Some(liked) = filter_liked {
            let likes = Query::select()
                .expr(Expr::val(1))
                .from(media_file_likes::Entity)
                .and_where(
                    Expr::col((
                        media_file_likes::Entity,
                        media_file_likes::Column::TrackUuid,
                    ))
                    .equals((media_files::Entity, media_files::Column::HlcUuid)),
                )
                .and_where(
                    Expr::col((media_file_likes::Entity, media_file_likes::Column::Liked)).eq(true),
                )
                .to_owned();
            filter = filter.add(if liked {
                Expr::exists(likes)
            } else {
                Expr::not_exists(likes)
            });
        }

        if let Some(analyzed) = filter_analyzed {
            let current = media_analysis::Entity::find()
                .select_only()
                .column(media_analysis::Column::FileId)
                .filter(
                    media_analysis::Column::AnalysisVersion.eq(analysis::CURRENT_ANALYSIS_VERSION),
                )
                .into_query();
            filter = filter.add(if analyzed {
                media_files::Column::Id.in_subquery(current)
            } else {
                media_files::Column::Id.not_in_subquery(current)
            });
        }

        if let Some(cover_art) = filter_cover_art {
            filter = filter.add(if cover_art {
                media_files::Column::CoverArtUuid.is_not_null()
            } else {
                media_files::Column::CoverArtUuid.is_null()
            });
        }

        query = query.filter(filter);
    } else if !all {
        query = query.filter(or_condition);
    }

    // Join with media_file_stats table for sorting by playedthrough and skipped, and filtering by liked
    query = apply_join_filter(
        query,
        filter_liked,
        filter_analyzed,
        sort_track_number_asc,
        sort_playedthrough_asc,
        sort_skipped_asc,
    );

    if only_one_playlist {
        query = query
            .join(
                JoinType::LeftJoin,
                media_file_playlists::Relation::MediaFiles
                    .def()
                    .rev()
                    .on_condition(move |_left, right| {
                        Expr::col((right, media_file_playlists::Column::PlaylistUuid))
                            .in_subquery(
                                playlists::Entity::find()
                                    .select_only()
                                    .column(playlists::Column::HlcUuid)
                                    .filter(playlists::Column::Id.eq(only_playlist))
                                    .into_query(),
                            )
                            .into_condition()
                    }),
            )
            .column(media_file_playlists::Column::Position);
    }

    if let Some(recommend_group) = pipe_recommend {
        apply_sorting_macro!(
            query,
            media_files::Column::LastModified,
            sort_last_modified_asc
        );
        apply_sorting_macro!(query, media_files::Column::Duration, sort_duration_asc);

        if let Some(asc) = sort_track_number_asc {
            query = query.order_by(
                media_file_albums::Column::TrackNumber,
                if asc { Order::Asc } else { Order::Desc },
            );
        }

        if let Some(asc) = sort_playedthrough_asc {
            query = query.order_by(
                statistics_sum(media_file_stats::Column::PlayedThrough),
                if asc { Order::Asc } else { Order::Desc },
            );
        }

        if let Some(asc) = sort_skipped_asc {
            query = query.order_by(
                statistics_sum(media_file_stats::Column::Skipped),
                if asc { Order::Asc } else { Order::Desc },
            );
        }

        if let Some(query_limit) = pipe_limit {
            query = query.limit(query_limit);
        }

        let candidate_file_ids = query
            .select_only()
            .column(media_files::Column::Id)
            .distinct()
            .into_tuple::<i32>()
            .all(main_db)
            .await
            .with_context(|| "Failed to query file ids for recommendation")?;

        if candidate_file_ids.is_empty() {
            return Ok([].to_vec());
        }

        let virtual_point: [f32; 61] = if recommend_group >= 0 {
            get_percentile_analysis_result(
                main_db,
                1.0 / (9 + 2) as f64 * (recommend_group + 1) as f64,
            )
            .await
            .with_context(|| "Failed to query percentile data")?
        } else {
            get_centralized_analysis_result(main_db, candidate_file_ids)
                .await
                .with_context(|| "Failed to query centralized data")?
                .into()
        };

        let recommend_n = pipe_limit.unwrap_or(30);

        let file_ids = match get_recommendation_by_parameter(
            recommend_db,
            virtual_point,
            recommend_n as usize,
        )
        .with_context(|| "Failed to get recommendation by parameters")
        {
            Ok(x) => x.into_iter().map(|x| x.0 as i32).collect::<Vec<i32>>(),
            Err(_) => return Ok([].to_vec()),
        };

        let media_files = get_files_by_ids(main_db, &file_ids).await?;

        // Create a hash map to store files by their ID
        let file_map: std::collections::HashMap<i32, _> = media_files
            .into_iter()
            .map(|file| (file.id, file))
            .collect();

        // Reorder files according to the order of file_ids
        let files_by_recommendation = file_ids
            .clone()
            .into_iter()
            .filter_map(|id| file_map.get(&id).cloned())
            .collect::<Vec<_>>();

        let sorted_files = sort_media_files(files_by_recommendation, &track_ids);

        return Ok(sorted_files);
    }

    if only_one_playlist {
        query = query.order_by(media_file_playlists::Column::Position, Order::Asc);
    }

    if let Some(asc) = sort_track_number_asc {
        query = query.order_by(
            media_file_albums::Column::TrackNumber,
            if asc { Order::Asc } else { Order::Desc },
        );
    }

    if let Some(asc) = sort_last_modified_asc {
        query = query.order_by(
            media_files::Column::LastModified,
            if asc { Order::Asc } else { Order::Desc },
        );
    }

    if let Some(asc) = sort_duration_asc {
        query = query.order_by(
            media_files::Column::Duration,
            if asc { Order::Asc } else { Order::Desc },
        );
    }

    if let Some(asc) = sort_playedthrough_asc {
        query = query.order_by(
            statistics_sum(media_file_stats::Column::PlayedThrough),
            if asc { Order::Asc } else { Order::Desc },
        );
    }

    if let Some(asc) = sort_skipped_asc {
        query = query.order_by(
            statistics_sum(media_file_stats::Column::Skipped),
            if asc { Order::Asc } else { Order::Desc },
        );
    }

    if let Some(limit) = pipe_limit
        && cursor as u64 >= limit
    {
        return Ok(vec![]);
    }

    let final_page_size = if let Some(limit) = pipe_limit {
        std::cmp::min(limit - cursor as u64, page_size as u64)
    } else {
        page_size as u64
    };

    let media_files = query
        .offset(Some(cursor as u64))
        .limit(final_page_size)
        .all(main_db)
        .await?;

    log::debug!(
        "query_mix_media_files: returning {} media files",
        media_files.len()
    );

    let sorted_files = sort_media_files(media_files, &track_ids);

    Ok(sorted_files)
}

/// Query parameters resolved for this device. Missing targets remain identifiable.
#[derive(Clone, Debug)]
pub struct MixQueryView {
    pub clause_id: Uuid,
    pub operator: String,
    pub parameter: String,
    pub target_uuid: Option<Uuid>,
    pub available: bool,
}
pub async fn query_views_on<C: sea_orm::ConnectionTrait>(
    db: &C,
    mix_uuid: &str,
) -> Result<Vec<MixQueryView>> {
    use crate::sync::model::{RuneModel, mix_target_collection};
    use sea_orm::{QueryOrder, QuerySelect};
    use sync::protocol::SyncModel;
    let rows = mix_queries::Entity::find()
        .filter(mix_queries::Column::MixUuid.eq(mix_uuid))
        .order_by_asc(mix_queries::Column::Position)
        .limit(129)
        .all(db)
        .await?;
    anyhow::ensure!(rows.len() <= 128, "mix exceeds clause limit");
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let target = row
            .target_uuid
            .as_deref()
            .map(str::parse::<Uuid>)
            .transpose()?;
        let (parameter, available) = if let Some(tag) = mix_target_collection(&row.operator) {
            let id = target.context("mix reference missing UUID")?;
            let collection =
                RuneModel::collection_from_tag(tag).context("unregistered mix reference")?;
            let local =
                RuneModel::local_id_on(db, &sync::protocol::ObjectKey { collection, id }).await?;
            (
                local.map_or_else(|| id.to_string(), |id| id.to_string()),
                local.is_some(),
            )
        } else if row.operator.starts_with("lib::directory.") {
            (
                row.local_directory.clone().unwrap_or_default(),
                row.local_directory.is_some(),
            )
        } else {
            (row.parameter, true)
        };
        result.push(MixQueryView {
            clause_id: row.hlc_uuid.parse()?,
            operator: row.operator,
            parameter,
            target_uuid: target,
            available,
        });
    }
    Ok(result)
}
pub async fn executable_queries_on<C: sea_orm::ConnectionTrait>(
    db: &C,
    mix_uuid: &str,
) -> Result<Vec<(String, String)>> {
    let views = query_views_on(db, mix_uuid).await?;
    anyhow::ensure!(
        views.iter().all(|view| view.available),
        "mix has unresolved references or unbound directories"
    );
    Ok(views
        .into_iter()
        .map(|view| (view.operator, view.parameter))
        .collect())
}

/// Resolve API row numbers once, then reconcile the complete ordered UUID list.
async fn replace_queries_on(
    tx: &DatabaseTransaction,
    mix_uuid: &str,
    queries: Vec<(String, String)>,
    group: i32,
) -> Result<()> {
    use crate::sync::model::{MixClause, MixClauseMap};
    use sync_orm::owned_list::{ListBudget, OwnedListPolicy};
    type Clauses = OwnedListPolicy<mix_queries::Entity, MixClauseMap>;
    anyhow::ensure!(queries.len() <= 128, "mix exceeds clause limit");
    let owner = [mix_uuid.to_owned().into()];
    let previous = Clauses::read_on(tx, &owner, Clauses::read_ceiling()).await?;
    let views = query_views_on(tx, mix_uuid).await?;
    let mut used = std::collections::HashSet::new();
    let mut incoming = Vec::with_capacity(queries.len());
    let mut directories = Vec::new();
    for (position, (operator, parameter)) in queries.into_iter().enumerate() {
        let target = resolve_target_on(tx, &operator, &parameter).await?;
        let directory = operator.starts_with("lib::directory.");
        let portable = if target.is_some() || directory {
            String::new()
        } else {
            parameter.clone()
        };
        let retained = previous
            .iter()
            .find(|clause| {
                !used.contains(&clause.id)
                    && clause.operator == operator
                    && clause.parameter == portable
                    && clause.target_uuid == target
                    && (!directory
                        || views
                            .iter()
                            .any(|view| view.clause_id == clause.id && view.parameter == parameter))
            })
            .or_else(|| {
                previous.get(position).filter(|clause| {
                    directory && clause.operator == operator && !used.contains(&clause.id)
                })
            });
        let id = retained.map_or_else(Uuid::new_v4, |clause| clause.id);
        used.insert(id);
        if directory {
            directories.push((id, parameter));
        }
        incoming.push(MixClause {
            id,
            position: i32::try_from(position)?,
            operator,
            parameter: portable,
            target_uuid: target,
            group,
        });
    }
    Clauses::prepare_on(
        tx,
        &owner,
        &incoming,
        ListBudget {
            max_reads: 129,
            max_mutations: 384,
        },
    )
    .await?
    .apply_on(tx)
    .await?;
    for (id, directory) in directories {
        mix_queries::Entity::update_many()
            .col_expr(
                mix_queries::Column::LocalDirectory,
                Expr::value(Some(directory)),
            )
            .filter(mix_queries::Column::HlcUuid.eq(id.to_string()))
            .filter(mix_queries::Column::MixUuid.eq(mix_uuid))
            .exec(tx)
            .await?;
    }
    Ok(())
}
async fn resolve_target_on(
    tx: &DatabaseTransaction,
    operator: &str,
    parameter: &str,
) -> Result<Option<Uuid>> {
    use crate::sync::model::{RuneModel, mix_target_collection};
    use sync::SyncModel;
    let Some(tag) = mix_target_collection(operator) else {
        return Ok(None);
    };
    if let Ok(id) = parameter.parse::<Uuid>() {
        return Ok(Some(id));
    }
    let local = parameter.parse::<i64>().context("invalid mix reference")?;
    let collection = RuneModel::collection_from_tag(tag).context("unregistered mix reference")?;
    Ok(Some(
        RuneModel::source_uuid_on(tx, &collection, local)
            .await?
            .context("mix reference not found")?,
    ))
}
async fn add_query_on(
    tx: &DatabaseTransaction,
    mix_uuid: &str,
    operator: String,
    parameter: String,
) -> Result<mix_queries::Model> {
    use crate::sync::model::{MixClause, MixClauseMap};
    use sync_orm::owned_list::{ListBudget, OwnedListPolicy};
    type Clauses = OwnedListPolicy<mix_queries::Entity, MixClauseMap>;
    let views = query_views_on(tx, mix_uuid).await?;
    let directory = operator.starts_with("lib::directory.");
    let existing = views.iter().find(|view| {
        view.operator == operator && view.parameter == parameter && (!directory || view.available)
    });
    let id = if let Some(existing) = existing {
        existing.clause_id
    } else {
        let owner = [mix_uuid.to_owned().into()];
        let mut clauses = Clauses::read_on(tx, &owner, Clauses::read_ceiling()).await?;
        anyhow::ensure!(clauses.len() < 128, "mix exceeds clause limit");
        let id = Uuid::new_v4();
        let target = resolve_target_on(tx, &operator, &parameter).await?;
        clauses.push(MixClause {
            id,
            position: i32::try_from(clauses.len())?,
            operator,
            parameter: if target.is_some() || directory {
                String::new()
            } else {
                parameter.clone()
            },
            target_uuid: target,
            group: 0,
        });
        Clauses::prepare_on(
            tx,
            &owner,
            &clauses,
            ListBudget {
                max_reads: 129,
                max_mutations: 384,
            },
        )
        .await?
        .apply_on(tx)
        .await?;
        if directory {
            mix_queries::Entity::update_many()
                .col_expr(
                    mix_queries::Column::LocalDirectory,
                    Expr::value(Some(parameter)),
                )
                .filter(mix_queries::Column::HlcUuid.eq(id.to_string()))
                .filter(mix_queries::Column::MixUuid.eq(mix_uuid))
                .exec(tx)
                .await?;
        }
        id
    };
    mix_queries::Entity::find()
        .filter(mix_queries::Column::HlcUuid.eq(id.to_string()))
        .filter(mix_queries::Column::MixUuid.eq(mix_uuid))
        .one(tx)
        .await?
        .context("mix clause disappeared")
}
