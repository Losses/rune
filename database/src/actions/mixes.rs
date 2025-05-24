use std::collections::{HashMap, HashSet};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use log::warn;
use migration::ExprTrait;
use migration::Func;
use migration::IntoCondition;
use migration::SimpleExpr;
use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{Condition, Expr};
use sea_orm::{
    ActiveValue, ColumnTrait, EntityTrait, JoinType, Order, QueryFilter, QueryOrder, QuerySelect,
    QueryTrait, TransactionTrait,
};

use crate::actions::analysis::get_analyze_count;
use crate::actions::analysis::get_percentile_analysis_result;
use crate::actions::cover_art::get_magic_cover_art_id;
use crate::actions::playback_queue::list_playback_queue;
use crate::connection::{MainDbConnection, RecommendationDbConnection};
use crate::entities::media_file_fingerprint;
use crate::entities::media_file_genres;
use crate::entities::{
    media_analysis, media_file_albums, media_file_artists, media_file_playlists, media_file_stats,
    media_files, mix_queries, mixes,
};

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
        Ok(get_mix_queries_by_mix_id(main_db, id)
            .await?
            .into_iter()
            .map(|x| (x.operator, x.parameter))
            .collect())
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
            .map_err(|e| anyhow::anyhow!("Failed to get collection groups: {}", e))
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
            sea_query::Func, sea_query::SimpleExpr, FromQueryResult, Order, QueryOrder,
            QuerySelect, QueryTrait,
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

pub async fn create_mix(
    db: &DatabaseConnection,
    node_id: &str,
    name: String,
    group: String,
    scriptlet_mode: bool,
    mode: i32,
    locked: bool,
) -> Result<mixes::Model> {
    use mixes::ActiveModel;

    let new_mix = ActiveModel {
        name: ActiveValue::Set(name),
        group: ActiveValue::Set(group),
        scriptlet_mode: ActiveValue::Set(scriptlet_mode),
        mode: ActiveValue::Set(Some(mode)),
        locked: ActiveValue::Set(locked),
        hlc_uuid: ActiveValue::Set(Uuid::new_v4().to_string()),
        created_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
        updated_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
        created_at_hlc_ver: ActiveValue::Set(0),
        updated_at_hlc_ver: ActiveValue::Set(0),
        created_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
        updated_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
        ..Default::default()
    };

    let inserted_mix = new_mix.insert(db).await?;
    Ok(inserted_mix)
}

pub async fn get_all_mixes(main_db: &DatabaseConnection) -> Result<Vec<mixes::Model>> {
    use mixes::Entity as MixEntity;

    let mixes = MixEntity::find().all(main_db).await?;
    Ok(mixes)
}

pub async fn get_mix_by_id(main_db: &DatabaseConnection, id: i32) -> Result<mixes::Model> {
    use mixes::Entity as MixEntity;

    let mix = MixEntity::find_by_id(id).one(main_db).await?;
    match mix {
        Some(m) => Ok(m),
        None => bail!("Mix not found"),
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn update_mix(
    db: &DatabaseConnection,
    node_id: &str,
    id: i32,
    name: Option<String>,
    group: Option<String>,
    scriptlet_mode: Option<bool>,
    mode: Option<i32>,
    locked: Option<bool>,
) -> Result<mixes::Model> {
    use mixes::Entity as MixEntity;

    let mix = MixEntity::find_by_id(id).one(db).await?;

    if let Some(mix) = mix {
        let ver = mix.created_at_hlc_ver;
        let mut active_model: mixes::ActiveModel = mix.into();

        if let Some(name) = name {
            active_model.name = ActiveValue::Set(name);
        }
        if let Some(group) = group {
            active_model.group = ActiveValue::Set(group);
        }
        if let Some(scriptlet_mode) = scriptlet_mode {
            active_model.scriptlet_mode = ActiveValue::Set(scriptlet_mode);
        }
        if let Some(mode) = mode {
            active_model.mode = ActiveValue::Set(Some(mode));
        }
        if let Some(locked) = locked {
            active_model.locked = ActiveValue::Set(locked);
        }

        active_model.updated_at_hlc_ts = ActiveValue::Set(Utc::now().to_rfc3339());
        active_model.updated_at_hlc_ver = ActiveValue::Set(ver + 1);
        active_model.updated_at_hlc_nid = ActiveValue::Set(node_id.to_owned());
        let updated_mix = active_model.update(db).await?;

        Ok(updated_mix)
    } else {
        bail!("Mix not found");
    }
}

pub async fn remove_mix(main_db: &DatabaseConnection, id: i32) -> Result<()> {
    use mixes::Entity as MixEntity;

    let mix = MixEntity::find_by_id(id).one(main_db).await?;
    if let Some(m) = mix {
        m.delete(main_db).await?;
        Ok(())
    } else {
        bail!("Mix not found")
    }
}

pub async fn replace_mix_queries(
    main_db: &DatabaseConnection,
    node_id: &str,
    mix_id: i32,
    operator_parameters: Vec<(String, String)>,
    group: Option<i32>,
) -> Result<()> {
    use mix_queries::Entity as MixQueryEntity;

    let txn = main_db.begin().await?;
    let mut existing_ids = Vec::new();

    for (operator, parameter) in &operator_parameters {
        let mix_query = MixQueryEntity::find()
            .filter(mix_queries::Column::MixId.eq(mix_id))
            .filter(mix_queries::Column::Operator.eq(operator))
            .filter(mix_queries::Column::Parameter.eq(parameter))
            .one(&txn)
            .await
            .with_context(|| {
                format!(
                    "Failed to query existed query with `{}({})`",
                    operator, parameter
                )
            })?;

        if let Some(existing_mix_query) = mix_query {
            existing_ids.push(existing_mix_query.id);
        } else {
            let mix_query = mix_queries::ActiveModel {
                mix_id: ActiveValue::Set(mix_id),
                operator: ActiveValue::Set(operator.clone()),
                parameter: ActiveValue::Set(parameter.clone()),
                group: ActiveValue::Set(group.unwrap_or_default()),
                hlc_uuid: ActiveValue::Set(Uuid::new_v4().to_string()),
                created_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
                updated_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
                created_at_hlc_ver: ActiveValue::Set(0),
                updated_at_hlc_ver: ActiveValue::Set(0),
                created_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
                updated_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
                ..Default::default()
            };

            mix_queries::Entity::insert(mix_query)
                .exec(&txn)
                .await
                .with_context(|| {
                    format!(
                        "Failed to insert new query with `{}({})`",
                        operator, parameter
                    )
                })?;
        };
    }

    let mut operator_parameter_conditions = Condition::any();
    for (operator, parameter) in &operator_parameters {
        operator_parameter_conditions = operator_parameter_conditions.add(
            Condition::all()
                .add(mix_queries::Column::Operator.eq(operator.clone()))
                .add(mix_queries::Column::Parameter.eq(parameter.clone())),
        );
    }

    let delete_condition = Condition::all()
        .add(mix_queries::Column::MixId.eq(mix_id))
        .add(Condition::not(operator_parameter_conditions));

    MixQueryEntity::delete_many()
        .filter(delete_condition)
        .exec(&txn)
        .await?;

    txn.commit().await?;

    Ok(())
}
pub async fn get_mix_queries_by_mix_id(
    main_db: &DatabaseConnection,
    mix_id: i32,
) -> Result<Vec<mix_queries::Model>> {
    use mix_queries::Entity as MixQueryEntity;

    Ok(MixQueryEntity::find()
        .filter(mix_queries::Column::MixId.eq(mix_id))
        .all(main_db)
        .await?)
}

pub async fn remove_mix_query(main_db: &DatabaseConnection, id: i32) -> Result<()> {
    use mix_queries::Entity as MixQueryEntity;

    let mix_query = MixQueryEntity::find_by_id(id).one(main_db).await?;
    if let Some(mq) = mix_query {
        mq.delete(main_db).await?;
        Ok(())
    } else {
        bail!("Mix query not found");
    }
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
            warn!(
                "Unable to parse the parameter of operator: {}({})",
                operator, parameter
            );
            None
        }
    }
}

pub async fn add_item_to_mix(
    main_db: &DatabaseConnection,
    node_id: &str,
    mix_id: i32,
    operator: String,
    parameter: String,
) -> Result<mix_queries::Model> {
    use mix_queries::ActiveModel;
    use mix_queries::Entity as MixQueryEntity;

    // Check for duplicates: check if there is already an entry with the same mix_id, operator, and parameter in the mix_queries table
    let existing_item = MixQueryEntity::find()
        .filter(mix_queries::Column::MixId.eq(mix_id))
        .filter(mix_queries::Column::Operator.eq(operator.clone()))
        .filter(mix_queries::Column::Parameter.eq(parameter.clone()))
        .one(main_db)
        .await?;

    if let Some(existing_item) = existing_item {
        // If the entry already exists, return the existing entry directly
        Ok(existing_item)
    } else {
        // If the entry does not exist, insert a new entry
        let new_mix_query = ActiveModel {
            mix_id: ActiveValue::Set(mix_id),
            operator: ActiveValue::Set(operator),
            parameter: ActiveValue::Set(parameter),
            group: ActiveValue::Set(0),
            hlc_uuid: ActiveValue::Set(Uuid::new_v4().to_string()),
            created_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
            updated_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
            created_at_hlc_ver: ActiveValue::Set(0),
            updated_at_hlc_ver: ActiveValue::Set(0),
            created_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
            updated_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
            ..Default::default()
        };

        let inserted_mix_query = new_mix_query.insert(main_db).await?;
        Ok(inserted_mix_query)
    }
}

pub async fn initialize_mix_queries(main_db: &DatabaseConnection, node_id: &str) -> Result<()> {
    let all_mixes: Vec<mixes::Model> = mixes::Entity::find()
        .filter(
            Condition::all()
                .add(mixes::Column::Name.starts_with("\u{200B}"))
                .add(mixes::Column::Group.eq("\u{200B}Rune"))
                .add(mixes::Column::Locked.eq(true))
                .add(mixes::Column::ScriptletMode.eq(false)),
        )
        .all(main_db)
        .await?;

    for mix in all_mixes {
        let mix_id = mix.id;

        let n = mix_queries::Entity::find()
            .filter(mix_queries::Column::MixId.eq(mix_id))
            .count(main_db)
            .await?;

        if n == 0 {
            let mut new_queries = vec![("lib::all", "true")];

            if mix.name == "\u{200B}Liked" {
                new_queries.push(("filter::liked", "true"));
            } else if mix.name.starts_with("\u{200B}Mix ") {
                if let Some(n) = mix.name.split_whitespace().last() {
                    new_queries.push(("pipe::limit", "50"));
                    new_queries.push(("pipe::recommend", n));
                }
            }

            for (operator, parameter) in new_queries {
                let new_mix_query = mix_queries::ActiveModel {
                    mix_id: ActiveValue::Set(mix_id),
                    operator: ActiveValue::Set(operator.to_string()),
                    parameter: ActiveValue::Set(parameter.to_string()),
                    group: ActiveValue::Set(0),
                    hlc_uuid: ActiveValue::Set(Uuid::new_v4().to_string()),
                    created_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
                    updated_at_hlc_ts: ActiveValue::Set(Utc::now().to_rfc3339()),
                    created_at_hlc_ver: ActiveValue::Set(0),
                    updated_at_hlc_ver: ActiveValue::Set(0),
                    created_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
                    updated_at_hlc_nid: ActiveValue::Set(node_id.to_owned()),
                    ..Default::default()
                };

                new_mix_query.insert(main_db).await?;
            }
        }
    }

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
    filter_liked: Option<bool>,
    filter_analyzed: Option<bool>,
    sort_track_number: Option<bool>,
    sort_playedthrough_asc: Option<bool>,
    sort_skipped_asc: Option<bool>,
) -> Select<media_files::Entity> {
    let mut _query = query;
    if filter_liked.is_some() || sort_playedthrough_asc.is_some() || sort_skipped_asc.is_some() {
        _query = _query
            .join(
                JoinType::LeftJoin,
                media_file_stats::Relation::MediaFiles.def().rev(),
            )
            .column(media_file_stats::Column::Liked)
            .column(media_file_stats::Column::PlayedThrough)
            .column(media_file_stats::Column::Skipped);
    }

    if filter_analyzed.is_some() {
        _query = _query
            .join(
                JoinType::LeftJoin,
                media_analysis::Relation::MediaFiles.def().rev(),
            )
            .column(media_analysis::Column::Id);
    }

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
    ($or_condition:expr, $ids:expr, $entity:ty, $column:expr, $file_column:expr) => {
        if !$ids.is_empty() {
            let subquery = <$entity>::find()
                .select_only()
                .filter($column.is_in($ids))
                .column($file_column)
                .into_query();

            $or_condition =
                $or_condition.add(Expr::cust("\"media_files\".\"id\"").in_subquery(subquery));
        }
    };
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
            QueryOperator::Unknown(op) => warn!("Unknown operator: {}", op),
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

    // Base query for media_files
    let mut query = media_files::Entity::find()
        .join(
            JoinType::LeftJoin,
            media_file_fingerprint::Relation::MediaFiles.def().rev(),
        )
        .filter(
            Condition::any()
                .add(media_file_fingerprint::Column::IsDuplicated.is_null())
                .add(media_file_fingerprint::Column::IsDuplicated.ne(1)),
        );

    // Create an OR condition to hold all the subconditions
    let mut or_condition = Condition::any();

    // Filter by artist_ids if provided
    add_subquery_filter!(
        or_condition,
        artist_ids,
        media_file_artists::Entity,
        media_file_artists::Column::ArtistId,
        media_file_artists::Column::MediaFileId
    );

    // Filter by album_ids if provided
    add_subquery_filter!(
        or_condition,
        album_ids,
        media_file_albums::Entity,
        media_file_albums::Column::AlbumId,
        media_file_albums::Column::MediaFileId
    );

    // Filter by genres_ids if provided
    add_subquery_filter!(
        or_condition,
        genre_ids,
        media_file_genres::Entity,
        media_file_genres::Column::GenreId,
        media_file_genres::Column::MediaFileId
    );

    // Filter by playlist_ids if provided
    add_subquery_filter!(
        or_condition,
        playlist_ids.clone(),
        media_file_playlists::Entity,
        media_file_playlists::Column::PlaylistId,
        media_file_playlists::Column::MediaFileId
    );

    // Filter by track_ids if provided
    if !track_ids.is_empty() {
        let subquery = media_files::Entity::find()
            .select_only()
            .filter(media_files::Column::Id.is_in(track_ids.clone()))
            .column(media_files::Column::Id)
            .into_query();

        or_condition = or_condition.add(Expr::cust("\"media_files\".\"id\"").in_subquery(subquery));
    }

    // Filter by directories if provided
    if !directories_deep.is_empty() {
        let mut dir_conditions = Condition::any();
        for dir in directories_deep {
            let dir = dir.strip_prefix('/').unwrap_or(&dir);

            dir_conditions = dir_conditions.add(
                Expr::col(media_files::Column::Directory)
                    .eq(dir)
                    .or(Expr::col(media_files::Column::Directory).like(format!("{}/%", dir))),
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

        or_condition = or_condition.add(Expr::cust("\"media_files\".\"id\"").in_subquery(subquery));
    }

    if let Some(queue_enabled) = playback_queue {
        if queue_enabled {
            let queued_tracks = list_playback_queue(main_db).await?;

            let subquery = media_files::Entity::find()
                .select_only()
                .filter(media_files::Column::Id.is_in(queued_tracks))
                .column(media_files::Column::Id)
                .into_query();

            or_condition =
                or_condition.add(Expr::cust("\"media_files\".\"id\"").in_subquery(subquery));
        }
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
            filter = filter.add(media_file_stats::Column::Liked.eq(liked));
        }

        if let Some(analyzed) = filter_analyzed {
            if analyzed {
                filter = filter.add(media_analysis::Column::Id.is_not_null());
            } else {
                filter = filter.add(media_analysis::Column::Id.is_null());
            }
        }

        if let Some(cover_art) = filter_cover_art {
            let magic_cover_art_id = get_magic_cover_art_id(main_db).await;

            if cover_art {
                let mut condition = Condition::all();
                condition = condition.add(media_files::Column::CoverArtId.is_not_null());

                if let Some(magic_cover_art_id) = magic_cover_art_id {
                    condition =
                        condition.add(media_files::Column::CoverArtId.ne(magic_cover_art_id));
                }
                filter = filter.add(condition);
            } else {
                let mut condition = Condition::any();
                condition = condition.add(media_files::Column::CoverArtId.is_null());

                if let Some(magic_cover_art_id) = magic_cover_art_id {
                    condition =
                        condition.add(media_files::Column::CoverArtId.eq(magic_cover_art_id));
                }
                filter = filter.add(condition);
            }
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
                        Expr::col((right, media_file_playlists::Column::PlaylistId))
                            .eq(only_playlist)
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
                media_file_stats::Column::PlayedThrough,
                if asc { Order::Asc } else { Order::Desc },
            );
        }

        if let Some(asc) = sort_skipped_asc {
            query = query.order_by(
                media_file_stats::Column::Skipped,
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
            media_file_stats::Column::PlayedThrough,
            if asc { Order::Asc } else { Order::Desc },
        );
    }

    if let Some(asc) = sort_skipped_asc {
        query = query.order_by(
            media_file_stats::Column::Skipped,
            if asc { Order::Asc } else { Order::Desc },
        );
    }

    if let Some(limit) = pipe_limit {
        if cursor as u64 >= limit {
            return Ok(vec![]);
        }
    }

    let final_page_size = if let Some(limit) = pipe_limit {
        (limit - cursor as u64).min(page_size as u64)
    } else {
        page_size as u64
    };

    let media_files = query
        .offset(Some(cursor as u64))
        .limit(final_page_size)
        .all(main_db)
        .await
        .unwrap();

    let sorted_files = sort_media_files(media_files, &track_ids);

    Ok(sorted_files)
}
