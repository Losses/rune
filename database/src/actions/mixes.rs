use crate::actions::artist_page::{artist_section_files, artist_track_files};
use crate::actions::resolved_metadata::queries::verified_audio;
use crate::actions::search::{add_term, remove_term};
use crate::error::ExpectedError;
use crate::mix_operator::{ArtistSection, MixOperator, UnknownOperator};
use crate::sync::{
    SeaOrmSyncStore,
    bindings::apply,
    model::{Collection, MixClauseMap, ObjectKey, author_source_on},
};
use ::playback::PlaybackMode;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use async_trait::async_trait;
use migration::{ExprTrait, Func, IntoCondition, SimpleExpr};
use sea_orm::entity::prelude::*;
use sea_orm::{
    ColumnTrait, DatabaseTransaction, EntityTrait, JoinType, Order, QueryFilter, QueryOrder,
    QuerySelect, QueryTrait, Set, TransactionTrait,
    sea_query::{BinOper, Condition, Expr, LikeExpr, Query, SelectStatement, UnionType},
};

use crate::actions::analysis::{
    current_analysis_condition, get_analyze_count, get_percentile_analysis_result,
};
use crate::connection::{MainDbConnection, RecommendationDbConnection};
use crate::entities::{
    audio_artists, genres, media_analysis, media_file_fingerprint, media_file_genres,
    media_file_likes, media_file_playlists, media_file_stats, media_files, mix_queries, mixes,
    playlists, track_album_sources,
};

use super::resolved_metadata::{column, nocase};
use crate::entities::music_metadata::{
    audio as catalog_audio, audio as verified, credits, discs, recording_credits, release_tracks,
    track_assets, values as catalog_values,
};

use super::analysis::get_centralized_analysis_result;
use super::collection::{CollectionQuery, CollectionQueryListMode, CollectionQueryType};
use super::recommendation::get_recommendation_by_parameter;
use super::utils::CollectionDefinition;
use std::cmp;
use std::collections;
use sync::protocol;
use sync_orm::causal_store;

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
                mixes::Entity::find()
                    .order_by(Expr::from(Func::random()), Order::Asc)
                    .limit(limit)
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
    author_source_on(
        tx,
        store,
        ObjectKey {
            collection: Collection::Mixes,
            id: mix.source_uuid.parse()?,
        },
    )
    .await
}

/// A stored mix mode that is not one of the known numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MixModeError {
    #[error("unknown stored mix playback mode {0}")]
    Unknown(i32),
}

// The stored and synced form of a mix's playback mode stays numeric, because changing it
// changes the sync journal format. These two functions are the only place the numbers exist;
// 99, and a missing value, stand for "keep the player's mode".
const STORED_KEEP_PLAYBACK_MODE: i32 = 99;

/// The number a mix's playback mode is stored and synced as.
pub fn stored_mix_mode(mode: Option<PlaybackMode>) -> i32 {
    match mode {
        Some(PlaybackMode::Sequential) => 0,
        Some(PlaybackMode::RepeatOne) => 1,
        Some(PlaybackMode::RepeatAll) => 2,
        Some(PlaybackMode::Shuffle) => 3,
        None => STORED_KEEP_PLAYBACK_MODE,
    }
}

/// The playback mode of a stored mix row, `None` when the mix keeps the player's mode.
pub fn mix_playback_mode(row: &mixes::Model) -> Result<Option<PlaybackMode>, MixModeError> {
    let Some(stored) = row.mode else {
        return Ok(None);
    };
    match stored {
        0 => Ok(Some(PlaybackMode::Sequential)),
        1 => Ok(Some(PlaybackMode::RepeatOne)),
        2 => Ok(Some(PlaybackMode::RepeatAll)),
        3 => Ok(Some(PlaybackMode::Shuffle)),
        STORED_KEEP_PLAYBACK_MODE => Ok(None),
        other => Err(MixModeError::Unknown(other)),
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn create_mix(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    name: String,
    group: String,
    mode: Option<PlaybackMode>,
    locked: bool,
    queries: Vec<(String, String)>,
) -> Result<mixes::Model> {
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let row = mixes::ActiveModel {
        name: Set(name),
        group: Set(group),
        // The flag is a synced field of the mix; no mix is a scriptlet, so it is always false.
        scriptlet_mode: Set(false),
        mode: Set(Some(stored_mix_mode(mode))),
        locked: Set(locked),
        source_uuid: Set(Uuid::new_v4().to_string()),
        ..Default::default()
    }
    .insert(&tx)
    .await?;
    add_term(&tx, CollectionQueryType::Mix, row.id, &row.name).await?;
    replace_queries_on(&tx, &row.source_uuid, queries, 0).await?;
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
        .ok_or(ExpectedError::NotFound { what: "mix" })
        .map_err(Into::into)
}

/// What an update does to a mix's playback mode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MixModeUpdate {
    /// Leave the stored mode as it is.
    Unchanged,
    /// Make the mix keep the player's mode.
    KeepPlayerMode,
    /// Make the mix play in this mode.
    Set(PlaybackMode),
}

#[allow(clippy::too_many_arguments)]
pub async fn update_mix(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    id: i32,
    name: Option<String>,
    group: Option<String>,
    mode: MixModeUpdate,
    locked: Option<bool>,
    queries: Option<Vec<(String, String)>>,
) -> Result<mixes::Model> {
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let row = mixes::Entity::find_by_id(id)
        .one(&tx)
        .await?
        .ok_or(ExpectedError::NotFound { what: "mix" })?;
    let mut model: mixes::ActiveModel = row.into();
    if let Some(v) = name {
        model.name = Set(v);
    }
    if let Some(v) = group {
        model.group = Set(v);
    }
    match mode {
        MixModeUpdate::Unchanged => {}
        MixModeUpdate::KeepPlayerMode => model.mode = Set(Some(stored_mix_mode(None))),
        MixModeUpdate::Set(v) => model.mode = Set(Some(stored_mix_mode(Some(v)))),
    }
    if let Some(v) = locked {
        model.locked = Set(v);
    }
    let row = model.update(&tx).await?;
    if let Some(queries) = queries {
        replace_queries_on(&tx, &row.source_uuid, queries, 0).await?;
    }
    add_term(&tx, CollectionQueryType::Mix, row.id, &row.name).await?;
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
        .ok_or(ExpectedError::NotFound { what: "mix" })?;
    let key = ObjectKey {
        collection: Collection::Mixes,
        id: row.source_uuid.parse()?,
    };
    remove_term(&tx, CollectionQueryType::Mix, id).await?;
    apply(&tx, &key, None).await?;
    store.author_on(&tx, key, None).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn replace_mix_queries(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    mix_id: i32,
    mix_source_uuid: &str,
    queries: Vec<(String, String)>,
    group: Option<i32>,
) -> Result<()> {
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let mix = mixes::Entity::find_by_id(mix_id)
        .one(&tx)
        .await?
        .ok_or(ExpectedError::NotFound { what: "mix" })?;
    anyhow::ensure!(mix.source_uuid == mix_source_uuid, "mix identity changed");
    replace_queries_on(&tx, &mix.source_uuid, queries, group.unwrap_or_default()).await?;
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
        .ok_or(ExpectedError::NotFound { what: "mix" })?;
    let mut views = query_views_on(db, &mix.source_uuid).await?;
    for view in &mut views {
        super::browse::editable_clause(db, view).await?;
    }
    Ok(views)
}
pub async fn executable_mix_queries(
    db: &DatabaseConnection,
    mix_id: i32,
) -> Result<Vec<(String, String)>> {
    let mix = mixes::Entity::find_by_id(mix_id)
        .one(db)
        .await?
        .ok_or(ExpectedError::NotFound { what: "mix" })?;
    executable_queries_on(db, &mix.source_uuid).await
}
pub async fn remove_mix_query(
    db: &DatabaseConnection,
    store: &SeaOrmSyncStore,
    id: i32,
) -> Result<()> {
    use sync_orm::owned_list::{ListBudget, OwnedListPolicy};
    type Clauses = OwnedListPolicy<mix_queries::Entity, MixClauseMap>;
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    let row = mix_queries::Entity::find_by_id(id)
        .one(&tx)
        .await?
        .ok_or(ExpectedError::NotFound { what: "mix clause" })?;
    let mix = mixes::Entity::find()
        .filter(mixes::Column::SourceUuid.eq(&row.mix_uuid))
        .one(&tx)
        .await?
        .ok_or(ExpectedError::NotFound { what: "mix" })?;
    let owner = [row.mix_uuid.into()];
    let mut items = Clauses::read_on(&tx, &owner, Clauses::read_ceiling()).await?;
    items.retain(|item| item.id.to_string() != row.source_uuid);
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryOperator {
    LibAll(bool),
    LibArtist(i32),
    LibArtistId(Uuid),
    LibAlbum(i32),
    LibRelease(Uuid),
    LibAlbumArtist(String),
    LibLabel(String),
    LibGenre(i32),
    LibPlaylist(i32),
    LibTrack(i32),
    LibRandom(i32),
    LibQueue(bool),
    LibDirectoryDeep(String),
    LibDirectoryShallow(String),
    /// An artist's number and the section of its page: the songs of the section's releases,
    /// release by release. A local library sorts its catalog's releases into the sections.
    LibArtistSection(i32, ArtistSection),
    /// An artist's number: the artist's songs, most listened first. It stands alone.
    LibArtistTracks(i32),
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
        .ok_or(ExpectedError::NotFound { what: "mix" })?;
    let row = add_query_on(&tx, &mix.source_uuid, operator, parameter).await?;
    author_mix(&tx, store, &mix).await?;
    tx.commit().await?;
    Ok(row)
}

/// Initialize built-in clauses only before a replication namespace exists.
pub async fn initialize_mix_queries_before_replication(db: &DatabaseConnection) -> Result<()> {
    let tx = db.begin().await?;
    anyhow::ensure!(
        causal_store::list_namespace_ids_on(&tx, 1)
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
            .filter(mix_queries::Column::MixUuid.eq(&mix.source_uuid))
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
            replace_queries_on(&tx, &mix.source_uuid, queries, 0).await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

/// The text of a clause is not a valid use of the mix query language.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QueryParseError {
    #[error(transparent)]
    UnknownOperator(#[from] UnknownOperator),
    #[error("invalid parameter `{parameter}` for mix query operator `{}`", operator.syntax())]
    InvalidParameter {
        operator: MixOperator,
        parameter: String,
    },
}

impl From<QueryParseError> for ExpectedError {
    fn from(error: QueryParseError) -> Self {
        ExpectedError::InvalidInput {
            detail: error.to_string(),
        }
    }
}

impl QueryOperator {
    /// Reads one clause of the syntax: an operator and its parameter text.
    pub fn parse(operator: &str, parameter: &str) -> Result<Self, QueryParseError> {
        let operator = MixOperator::from_syntax(operator)?;
        let invalid = || QueryParseError::InvalidParameter {
            operator,
            parameter: parameter.to_owned(),
        };
        let flag = || parameter.parse::<bool>().map_err(|_| invalid());
        let number = || parameter.parse::<i32>().map_err(|_| invalid());
        let text = || {
            let value = parameter.trim();
            if value.is_empty() {
                Err(invalid())
            } else {
                Ok(value.to_owned())
            }
        };
        Ok(match operator {
            MixOperator::LibAll => Self::LibAll(flag()?),
            MixOperator::LibArtist => Self::LibArtist(number()?),
            MixOperator::LibArtistId => {
                Self::LibArtistId(parameter.parse().map_err(|_| invalid())?)
            }
            MixOperator::LibAlbum => Self::LibAlbum(number()?),
            MixOperator::LibRelease => Self::LibRelease(parameter.parse().map_err(|_| invalid())?),
            MixOperator::LibAlbumArtist => Self::LibAlbumArtist(text()?),
            MixOperator::LibLabel => Self::LibLabel(text()?),
            MixOperator::LibGenre => Self::LibGenre(number()?),
            MixOperator::LibPlaylist => Self::LibPlaylist(number()?),
            MixOperator::LibTrack => Self::LibTrack(number()?),
            MixOperator::LibRandom => Self::LibRandom(number()?),
            MixOperator::LibQueue => Self::LibQueue(flag()?),
            MixOperator::LibDirectoryDeep => Self::LibDirectoryDeep(parameter.to_owned()),
            MixOperator::LibDirectoryShallow => Self::LibDirectoryShallow(parameter.to_owned()),
            MixOperator::LibArtistSection => {
                let (artist, section) = parameter.split_once(':').ok_or_else(invalid)?;
                Self::LibArtistSection(
                    artist.parse().map_err(|_| invalid())?,
                    ArtistSection::from_syntax(section).ok_or_else(invalid)?,
                )
            }
            MixOperator::LibArtistTracks => Self::LibArtistTracks(number()?),
            MixOperator::SortTrackNumber => Self::SortTrackNumber(flag()?),
            MixOperator::SortLastModified => Self::SortLastModified(flag()?),
            MixOperator::SortDuration => Self::SortDuration(flag()?),
            MixOperator::SortPlayedthrough => Self::SortPlayedthrough(flag()?),
            MixOperator::SortSkipped => Self::SortSkipped(flag()?),
            MixOperator::FilterLiked => Self::FilterLiked(flag()?),
            MixOperator::FilterWithCoverArt => Self::FilterWithCoverArt(flag()?),
            MixOperator::FilterAnalyzed => Self::FilterAnalyzed(flag()?),
            MixOperator::PipeLimit => {
                Self::PipeLimit(parameter.parse::<u64>().map_err(|_| invalid())?)
            }
            MixOperator::PipeRecommend => Self::PipeRecommend(number()?),
        })
    }

    pub fn operator(&self) -> MixOperator {
        match self {
            Self::LibAll(_) => MixOperator::LibAll,
            Self::LibArtist(_) => MixOperator::LibArtist,
            Self::LibArtistId(_) => MixOperator::LibArtistId,
            Self::LibAlbum(_) => MixOperator::LibAlbum,
            Self::LibRelease(_) => MixOperator::LibRelease,
            Self::LibAlbumArtist(_) => MixOperator::LibAlbumArtist,
            Self::LibLabel(_) => MixOperator::LibLabel,
            Self::LibGenre(_) => MixOperator::LibGenre,
            Self::LibPlaylist(_) => MixOperator::LibPlaylist,
            Self::LibTrack(_) => MixOperator::LibTrack,
            Self::LibRandom(_) => MixOperator::LibRandom,
            Self::LibQueue(_) => MixOperator::LibQueue,
            Self::LibDirectoryDeep(_) => MixOperator::LibDirectoryDeep,
            Self::LibDirectoryShallow(_) => MixOperator::LibDirectoryShallow,
            Self::LibArtistSection(..) => MixOperator::LibArtistSection,
            Self::LibArtistTracks(_) => MixOperator::LibArtistTracks,
            Self::SortTrackNumber(_) => MixOperator::SortTrackNumber,
            Self::SortLastModified(_) => MixOperator::SortLastModified,
            Self::SortDuration(_) => MixOperator::SortDuration,
            Self::SortPlayedthrough(_) => MixOperator::SortPlayedthrough,
            Self::SortSkipped(_) => MixOperator::SortSkipped,
            Self::FilterLiked(_) => MixOperator::FilterLiked,
            Self::FilterWithCoverArt(_) => MixOperator::FilterWithCoverArt,
            Self::FilterAnalyzed(_) => MixOperator::FilterAnalyzed,
            Self::PipeLimit(_) => MixOperator::PipeLimit,
            Self::PipeRecommend(_) => MixOperator::PipeRecommend,
        }
    }

    /// The clause as the syntax writes it.
    pub fn syntax(&self) -> (&'static str, String) {
        let parameter = match self {
            Self::LibAll(value)
            | Self::LibQueue(value)
            | Self::SortTrackNumber(value)
            | Self::SortLastModified(value)
            | Self::SortDuration(value)
            | Self::SortPlayedthrough(value)
            | Self::SortSkipped(value)
            | Self::FilterLiked(value)
            | Self::FilterWithCoverArt(value)
            | Self::FilterAnalyzed(value) => value.to_string(),
            Self::LibArtist(id)
            | Self::LibAlbum(id)
            | Self::LibGenre(id)
            | Self::LibPlaylist(id)
            | Self::LibTrack(id)
            | Self::LibRandom(id)
            | Self::LibArtistTracks(id)
            | Self::PipeRecommend(id) => id.to_string(),
            Self::LibDirectoryDeep(path) | Self::LibDirectoryShallow(path) => path.clone(),
            Self::LibRelease(id) | Self::LibArtistId(id) => id.to_string(),
            Self::LibAlbumArtist(value) | Self::LibLabel(value) => value.clone(),
            Self::LibArtistSection(artist, section) => format!("{artist}:{}", section.syntax()),
            Self::PipeLimit(limit) => limit.to_string(),
        };
        (self.operator().syntax(), parameter)
    }
}

fn catalog_track_number() -> Expr {
    use catalog_audio::Column as Audio;
    let disc: Expr = Func::coalesce([column(Audio::DiscNumber), Expr::val(0)]).into();
    let position = disc.mul(1000).add(column(Audio::TrackNumber));
    let query = Query::select()
        .expr(position)
        .from(catalog_audio::Entity)
        .and_where(column(Audio::FileId).eq(column(media_files::Column::Id)))
        .to_owned();
    Expr::SubQuery(None, Box::new(query.into()))
}

fn album_alias_condition(ids: Vec<i32>) -> Expr {
    // Resolve older public album IDs to catalog tracks using their portable identities.
    use crate::entities::portable_albums as editions;
    use catalog_audio::Column as Audio;
    use editions::Column as Edition;
    use track_album_sources::Column as Link;
    let uuids = Query::select()
        .column(Edition::SourceUuid)
        .from(editions::Entity)
        .and_where(Edition::Id.is_in(ids))
        .to_owned();
    let linked = Query::select()
        .column(Link::TrackUuid)
        .from(track_album_sources::Entity)
        .and_where(column(Link::AlbumUuid).in_subquery(uuids.clone()))
        .to_owned();

    // Read membership from the audio's resolved edition, including a migrated edition UUID.
    let tracks = Query::select()
        .column(Audio::FileId.as_column_ref())
        .from(catalog_audio::Entity)
        .inner_join(
            discs::Entity,
            column(Audio::DiscId).eq(column(discs::Column::Id)),
        )
        .and_where(
            column(discs::Column::AlbumId)
                .in_subquery(uuids)
                .or(column(Audio::Id).in_subquery(linked)),
        )
        .to_owned();
    column(media_files::Column::Id).in_subquery(tracks)
}

enum AlbumProperty {
    Artist,
    Label,
}

fn album_property_condition(property: AlbumProperty, name: &str) -> SimpleExpr {
    use catalog_audio::Column as A;
    use credits::Column as C;
    use discs::Column as D;

    // Each branch uses its own covering index to locate contributing tracks.
    let albums = match property {
        AlbumProperty::Artist => Query::select()
            .column(D::AlbumId.as_column_ref())
            .from(credits::Entity)
            .inner_join(
                catalog_audio::Entity,
                column(A::FileId).eq(column(C::FileId)),
            )
            .inner_join(discs::Entity, column(D::Id).eq(column(A::DiscId)))
            .and_where(C::Scope.eq(0))
            .and_where(C::Role.eq("Primary Artist"))
            .and_where(nocase(column(C::CreditedAs)).eq(name))
            .to_owned(),
        AlbumProperty::Label => labeled_albums(name),
    };
    let files = Query::select()
        .column(A::FileId.as_column_ref())
        .from(discs::Entity)
        .inner_join(catalog_audio::Entity, column(A::DiscId).eq(column(D::Id)))
        .and_where(column(D::AlbumId).in_subquery(albums))
        .to_owned();
    column(media_files::Column::Id).in_subquery(files)
}

fn labeled_albums(name: &str) -> SelectStatement {
    use catalog_audio::Column as A;
    use catalog_values::Column as V;
    use discs::Column as D;
    Query::select()
        .column(D::AlbumId.as_column_ref())
        .from(catalog_values::Entity)
        .inner_join(
            catalog_audio::Entity,
            column(A::FileId).eq(column(V::FileId)),
        )
        .inner_join(discs::Entity, column(D::Id).eq(column(A::DiscId)))
        .and_where(V::Scope.eq(0))
        .and_where(V::Field.eq("labels"))
        .and_where(V::Qualifier.eq(""))
        .and_where(nocase(column(V::Value)).eq(name))
        .to_owned()
}

fn artist_condition(ids: Vec<i32>) -> SimpleExpr {
    use crate::entities::portable_artists as artists;
    let targets = Query::select()
        .column(artists::Column::SourceUuid)
        .from(artists::Entity)
        .and_where(artists::Column::Id.is_in(ids))
        .to_owned();
    artist_tracks_condition(targets)
}

pub(super) fn artist_uuid_condition(id: Uuid) -> SimpleExpr {
    let targets = Query::select().expr(Expr::val(id.to_string())).to_owned();
    artist_tracks_condition(targets)
}

fn artist_tracks_condition(targets: SelectStatement) -> SimpleExpr {
    use audio_artists::Column as L;
    use catalog_audio::Column as A;
    use credits::Column as C;
    use recording_credits::Column as G;

    // Catalog artist links, file credits and recording credits share the same UUID.
    let matches = |artist: Expr| artist.in_subquery(targets.clone());
    let recorded = Query::select()
        .column(A::Id.as_column_ref())
        .from(recording_credits::Entity)
        .inner_join(
            catalog_audio::Entity,
            column(A::RecordingId).eq(column(G::RecordingId)),
        )
        .and_where(matches(column(G::ArtistId)))
        .to_owned();
    let credited = Query::select()
        .column(A::Id.as_column_ref())
        .from(credits::Entity)
        .inner_join(
            catalog_audio::Entity,
            column(A::FileId).eq(column(C::FileId)),
        )
        .and_where(C::Scope.eq(2))
        .and_where(matches(column(C::ArtistId)))
        .to_owned();
    let tracks = Query::select()
        .column(L::TrackUuid)
        .from(audio_artists::Entity)
        .and_where(matches(column(L::ArtistUuid)))
        .union(UnionType::Distinct, credited)
        .union(UnionType::Distinct, recorded)
        .to_owned();
    column(media_files::Column::SourceUuid).in_subquery(tracks)
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
                .column($parent::Column::SourceUuid)
                .filter($parent::Column::Id.is_in($ids))
                .into_query();
            let subquery = <$entity>::find()
                .select_only()
                .filter($column.in_subquery(parents))
                .column($file_column)
                .into_query();

            $or_condition = $or_condition.add(
                Expr::col((media_files::Entity, media_files::Column::SourceUuid))
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
            .equals((media_files::Entity, media_files::Column::SourceUuid)),
        )
        .to_owned();
    Func::coalesce([Expr::from(sum), Expr::val(0)]).into()
}

/// Keep a file unless it has a verified current duplicate mark: one whose
/// fingerprint matches the algorithm, configuration, format, and file content
/// checked below. NOT EXISTS avoids repeating a file for each matching
/// fingerprint row.
pub(super) fn not_verified_duplicate() -> SimpleExpr {
    let duplicates = Query::select()
        .expr(Expr::val(1))
        .from(media_file_fingerprint::Entity)
        .and_where(
            Expr::col((
                media_file_fingerprint::Entity,
                media_file_fingerprint::Column::TrackUuid,
            ))
            .equals((media_files::Entity, media_files::Column::SourceUuid)),
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
    Expr::not_exists(duplicates)
}

fn mix_asset_eligibility(
    filter_liked: Option<bool>,
    filter_cover_art: Option<bool>,
    filter_analyzed: Option<bool>,
) -> Condition {
    let mut filter = Condition::all().add(not_verified_duplicate());
    if let Some(liked) = filter_liked {
        let likes = Query::select()
            .expr(Expr::val(1))
            .from(media_file_likes::Entity)
            .and_where(
                Expr::col((
                    media_file_likes::Entity,
                    media_file_likes::Column::TrackUuid,
                ))
                .equals((media_files::Entity, media_files::Column::SourceUuid)),
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
            .join(
                JoinType::InnerJoin,
                media_analysis::Relation::MediaFiles.def(),
            )
            .select_only()
            .column(media_analysis::Column::FileId)
            .filter(current_analysis_condition())
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
    filter
}

fn release_condition(id: Uuid, eligibility: Condition) -> SimpleExpr {
    use media_files::Column as F;
    use release_tracks::Column as T;
    use track_assets::Column as V;
    use verified::Column as A;
    let codec: Expr = Func::lower(column(A::Codec)).into();
    let lossless = Func::coalesce([
        codec.is_in(["flac", "alac", "wav", "pcm", "aiff", "dsd", "wavpack"]),
        Expr::val(0),
    ]);

    // Apply eligibility before ranking variants and keep the same filter on the Mix output.
    let candidate = Query::select()
        .column(F::Id.as_column_ref())
        .from(media_files::Entity)
        .inner_join(
            track_assets::Entity,
            column(V::FileId)
                .eq(column(F::Id))
                .and(column(V::TrackId).eq(column(T::Id))),
        )
        .inner_join(verified_audio(), column(A::FileId).eq(column(F::Id)))
        .cond_where(eligibility)
        .order_by(F::LocalAvailable.as_column_ref(), Order::Desc)
        .order_by_expr(lossless.into(), Order::Desc)
        .order_by_columns(
            [A::BitsPerSample, A::SampleRate, A::Bitrate].map(|c| (c.as_column_ref(), Order::Desc)),
        )
        .order_by(F::Id.as_column_ref(), Order::Asc)
        .limit(1)
        .to_owned();
    let assets = Query::select()
        .expr(Expr::from(candidate))
        .from(release_tracks::Entity)
        .inner_join(
            discs::Entity,
            column(discs::Column::Id).eq(column(T::DiscId)),
        )
        .and_where(
            column(discs::Column::AlbumId).in_subquery(super::browse::edition_ids(&id.to_string())),
        )
        .to_owned();
    column(F::Id).in_subquery(assets)
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
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
    });

    media_files
}

/// The escape character of the LIKE patterns built by [`escape_like`].
const LIKE_ESCAPE: char = '\\';

/// Escapes `text` so it matches only itself inside a LIKE pattern that uses [`LIKE_ESCAPE`].
fn escape_like(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, LIKE_ESCAPE | '%' | '_') {
            escaped.push(LIKE_ESCAPE);
        }
        escaped.push(c);
    }
    escaped
}

/// An artist page query, which a library answers by itself.
enum ArtistPageQuery {
    Section(i32, ArtistSection),
    Tracks(i32),
}

pub async fn query_mix_media_files(
    main_db: &DatabaseConnection,
    recommend_db: &RecommendationDbConnection,
    queries: Vec<(String, String)>,
    cursor: usize,
    page_size: usize,
    session_queue: &[i32],
) -> Result<Vec<media_files::Model>> {
    query_mix_media_files_with_seed(
        main_db,
        recommend_db,
        queries,
        cursor,
        page_size,
        session_queue,
        None,
    )
    .await
}

/// Query a page of a mix, using the same random ordering for a supplied seed.
/// Reuse the seed across pages to avoid drawing a different random subset.
pub async fn query_mix_media_files_with_seed(
    main_db: &DatabaseConnection,
    recommend_db: &RecommendationDbConnection,
    queries: Vec<(String, String)>,
    cursor: usize,
    page_size: usize,
    session_queue: &[i32],
    random_seed: Option<u64>,
) -> Result<Vec<media_files::Model>> {
    // Edition clauses need current release identities and logical track assets.
    if queries.iter().any(|(operator, parameter)| {
        operator == MixOperator::LibRelease.syntax()
            || (operator == MixOperator::LibAlbum.syntax()
                && parameter.parse::<i32>().is_ok_and(|id| id < 0))
    }) {
        super::resolved_metadata::ensure_tracks_current(main_db).await?;
    }
    let mut all: bool = false;

    let mut artist_ids: Vec<i32> = vec![];
    let mut album_ids: Vec<i32> = vec![];
    let mut release_ids = Vec::new();
    let mut resolved_conditions = Vec::new();
    let mut genre_ids: Vec<i32> = vec![];
    let mut playlist_ids: Vec<i32> = vec![];
    let mut track_ids: Vec<i32> = vec![];
    let mut random_count: Vec<i32> = vec![];
    let mut directories_deep: Vec<String> = vec![];
    let mut directories_shallow: Vec<String> = vec![];
    let mut playback_queue: Option<bool> = None;
    let mut artist_page: Option<ArtistPageQuery> = None;

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

    // Clauses other than the cover filter, which an artist page query may come with.
    let mut other_clauses = 0;
    for (operator, parameter) in queries {
        let query = super::browse::portable_clause(main_db, operator, parameter).await?;
        let parsed = QueryOperator::parse(&query.0, &query.1).map_err(ExpectedError::from)?;
        if parsed != QueryOperator::FilterWithCoverArt(true) {
            other_clauses += 1;
        }
        match parsed {
            QueryOperator::LibAll(is_all) => all = is_all,
            QueryOperator::LibArtist(id) => artist_ids.push(id),
            QueryOperator::LibArtistId(id) => resolved_conditions.push(artist_uuid_condition(id)),
            QueryOperator::LibAlbum(id) => album_ids.push(id),
            QueryOperator::LibRelease(id) => release_ids.push(id),
            QueryOperator::LibAlbumArtist(name) => {
                resolved_conditions.push(album_property_condition(AlbumProperty::Artist, &name))
            }
            QueryOperator::LibLabel(name) => {
                resolved_conditions.push(album_property_condition(AlbumProperty::Label, &name))
            }
            QueryOperator::LibGenre(id) => genre_ids.push(id),
            QueryOperator::LibPlaylist(id) => playlist_ids.push(id),
            QueryOperator::LibTrack(id) => track_ids.push(id),
            QueryOperator::LibRandom(count) => random_count.push(count),
            QueryOperator::LibQueue(enabled) => playback_queue = Some(enabled),
            QueryOperator::LibDirectoryDeep(dir) => directories_deep.push(dir),
            QueryOperator::LibDirectoryShallow(dir) => directories_shallow.push(dir),
            QueryOperator::LibArtistSection(artist, section) => {
                artist_page = Some(ArtistPageQuery::Section(artist, section));
            }
            QueryOperator::LibArtistTracks(artist) => {
                artist_page = Some(ArtistPageQuery::Tracks(artist));
            }
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
        }
    }

    if let Some(page) = artist_page {
        // An artist's page lists in its own order, so nothing else can be combined with it,
        // except the filter for songs with cover art.
        if other_clauses != 1 {
            return Err(ExpectedError::InvalidInput {
                detail: "an artist page query stands alone".to_owned(),
            }
            .into());
        }
        let cover_only = filter_cover_art == Some(true);
        return match page {
            ArtistPageQuery::Section(artist, section) => {
                artist_section_files(main_db, artist, section, cover_only, cursor, page_size).await
            }
            ArtistPageQuery::Tracks(artist) => {
                artist_track_files(main_db, artist, cover_only, cursor, page_size).await
            }
        };
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
        && release_ids.is_empty()
        && resolved_conditions.is_empty()
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

    let eligibility = mix_asset_eligibility(filter_liked, filter_cover_art, filter_analyzed);
    let mut query = media_files::Entity::find().filter(eligibility.clone());
    if !all {
        resolved_conditions.extend(
            release_ids
                .into_iter()
                .map(|id| release_condition(id, eligibility.clone())),
        );
    }

    // Create an OR condition to hold all the subconditions
    let mut or_condition = Condition::any();
    for condition in resolved_conditions {
        or_condition = or_condition.add(condition);
    }

    if !artist_ids.is_empty() {
        or_condition = or_condition.add(artist_condition(artist_ids));
    }

    if !album_ids.is_empty() {
        or_condition = or_condition.add(album_alias_condition(album_ids));
    }

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
    if directories_deep.iter().any(|dir| dir == "/") {
        // Every library-relative directory is within the root.
        all = true;
    } else if !directories_deep.is_empty() {
        let mut dir_conditions = Condition::any();
        for dir in directories_deep {
            let dir = dir.strip_prefix('/').unwrap_or(&dir);

            let descendants = LikeExpr::new(format!("{}/%", escape_like(dir))).escape(LIKE_ESCAPE);
            dir_conditions = dir_conditions.add(
                Expr::col(media_files::Column::Directory)
                    .eq(dir)
                    .or(Expr::col(media_files::Column::Directory).like(descendants)),
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
        let order = match random_seed {
            Some(seed) => {
                // Both residues and their product fit in SQLite's signed integers.
                const PRIME: u64 = 2_147_483_647;
                let multiplier = ((seed >> 32) % (PRIME - 1) + 1) as i64;
                let offset = (seed % PRIME) as i64;
                let residue = column(media_files::Column::Id)
                    .mul(multiplier)
                    .add(offset)
                    .binary(BinOper::Mod, Expr::val(PRIME as i64));
                residue
                    .clone()
                    .mul(residue)
                    .binary(BinOper::Mod, Expr::val(PRIME as i64))
            }
            None => SimpleExpr::FunctionCall(Func::random()),
        };
        let subquery = media_files::Entity::find()
            .select_only()
            .order_by(order, Order::Asc)
            .order_by_asc(media_files::Column::Id)
            .limit(*random_count.iter().max().unwrap_or(&30) as u64)
            .column(media_files::Column::Id)
            .into_query();

        or_condition = or_condition
            .add(Expr::col((media_files::Entity, media_files::Column::Id)).in_subquery(subquery));
    }

    if let Some(queue_enabled) = playback_queue
        && queue_enabled
    {
        let subquery = media_files::Entity::find()
            .select_only()
            .filter(media_files::Column::Id.is_in(session_queue.to_vec()))
            .column(media_files::Column::Id)
            .into_query();

        or_condition = or_condition
            .add(Expr::col((media_files::Entity, media_files::Column::Id)).in_subquery(subquery));
    }

    if !all {
        query = query.filter(or_condition);
    }

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
                                    .column(playlists::Column::SourceUuid)
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
                catalog_track_number(),
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

        let file_ids = recommendation_file_ids(
            get_recommendation_by_parameter(recommend_db, virtual_point, recommend_n as usize)
                .with_context(|| "Failed to get recommendation by parameters"),
        )?;

        let media_files = media_files::Entity::find()
            .filter(media_files::Column::Id.is_in(file_ids.iter().copied()))
            .filter(not_verified_duplicate())
            .all(main_db)
            .await?;

        // Create a hash map to store files by their ID
        let file_map: collections::HashMap<i32, _> = media_files
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
            catalog_track_number(),
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
        cmp::min(limit - cursor as u64, page_size as u64)
    } else {
        page_size as u64
    };

    query = query.order_by_asc(media_files::Column::Id);

    let sorted_files = if track_ids.is_empty() {
        query
            .offset(Some(cursor as u64))
            .limit(final_page_size)
            .all(main_db)
            .await?
    } else {
        let media_files = query.all(main_db).await?;
        let mut sorted = sort_media_files(media_files, &track_ids);
        if let Some(limit) = pipe_limit {
            sorted.truncate(limit as usize);
        }
        sorted
            .into_iter()
            .skip(cursor)
            .take(final_page_size as usize)
            .collect()
    };

    log::debug!(
        "query_mix_media_files: returning {} media files",
        sorted_files.len()
    );

    Ok(sorted_files)
}

fn recommendation_file_ids(recommendations: Result<Vec<(u32, f32)>>) -> Result<Vec<i32>> {
    Ok(recommendations?
        .into_iter()
        .map(|(file_id, _)| file_id as i32)
        .collect())
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
// A stored or synced clause keeps its operator as text, because changing that changes the sync
// journal format. This is where the text becomes a `MixOperator` for the storage code.
fn stored_operator(text: &str) -> Result<MixOperator, ExpectedError> {
    MixOperator::from_syntax(text).map_err(|error| ExpectedError::InvalidInput {
        detail: error.to_string(),
    })
}

pub async fn query_views_on<C: sea_orm::ConnectionTrait>(
    db: &C,
    mix_uuid: &str,
) -> Result<Vec<MixQueryView>> {
    use crate::sync::model::RuneModel;
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
        let operator = stored_operator(&row.operator)?;
        let (parameter, available) = if let Some(target_kind) = operator.target_collection() {
            let id = target.context("mix reference missing UUID")?;
            let collection = RuneModel::collection_from_tag(target_kind.tag())
                .context("unregistered mix reference")?;
            let local = RuneModel::local_id_on(db, &protocol::ObjectKey { collection, id }).await?;
            (
                local.map_or_else(|| id.to_string(), |id| id.to_string()),
                local.is_some(),
            )
        } else if operator.is_directory() {
            (
                row.local_directory.clone().unwrap_or_default(),
                row.local_directory.is_some(),
            )
        } else {
            (row.parameter, true)
        };
        result.push(MixQueryView {
            clause_id: row.source_uuid.parse()?,
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
    let mut used = HashSet::new();
    let mut incoming = Vec::with_capacity(queries.len());
    let mut directories = Vec::new();
    for (position, (operator, parameter)) in queries.into_iter().enumerate() {
        let (operator, parameter) = super::browse::portable_clause(tx, operator, parameter).await?;
        let kind = mix_clause_operator(&operator)?;
        let target = resolve_target_on(tx, kind, &parameter).await?;
        let directory = kind.is_directory();
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
            .filter(mix_queries::Column::SourceUuid.eq(id.to_string()))
            .filter(mix_queries::Column::MixUuid.eq(mix_uuid))
            .exec(tx)
            .await?;
    }
    Ok(())
}
async fn resolve_target_on(
    tx: &DatabaseTransaction,
    operator: MixOperator,
    parameter: &str,
) -> Result<Option<Uuid>> {
    use crate::sync::model::RuneModel;
    use sync::SyncModel;
    let Some(target_kind) = operator.target_collection() else {
        return Ok(None);
    };
    let tag = target_kind.tag();
    if let Ok(id) = parameter.parse::<Uuid>() {
        return Ok(Some(id));
    }
    let local = parameter.parse::<i64>().context("invalid mix reference")?;
    let collection = RuneModel::collection_from_tag(tag).context("unregistered mix reference")?;
    Ok(Some(
        RuneModel::source_uuid_on(tx, &collection, local)
            .await?
            .ok_or(ExpectedError::NotFound {
                what: "mix reference",
            })?,
    ))
}
/// The operator of a clause a mix is given. A mix is synced by what it names, so other devices
/// play it too. The lists of an artist's page name sections a device derives from its own tags
/// and play counts, which another device may not share or may not know, so a mix cannot hold
/// them, as an offline rule cannot.
fn mix_clause_operator(text: &str) -> Result<MixOperator, ExpectedError> {
    let operator = stored_operator(text)?;
    if matches!(
        operator,
        MixOperator::LibArtistSection | MixOperator::LibArtistTracks
    ) {
        return Err(ExpectedError::InvalidInput {
            detail: format!("mix query operator `{text}` only lists an artist's page"),
        });
    }
    Ok(operator)
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
    let (operator, parameter) = super::browse::portable_clause(tx, operator, parameter).await?;
    let kind = mix_clause_operator(&operator)?;
    let directory = kind.is_directory();
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
        let target = resolve_target_on(tx, kind, &parameter).await?;
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
                .filter(mix_queries::Column::SourceUuid.eq(id.to_string()))
                .filter(mix_queries::Column::MixUuid.eq(mix_uuid))
                .exec(tx)
                .await?;
        }
        id
    };
    mix_queries::Entity::find()
        .filter(mix_queries::Column::SourceUuid.eq(id.to_string()))
        .filter(mix_queries::Column::MixUuid.eq(mix_uuid))
        .one(tx)
        .await?
        .context("mix clause disappeared")
}

#[cfg(test)]
mod recommendation_failure_tests {
    use super::recommendation_file_ids;

    #[test]
    fn recommendation_lookup_failure_is_propagated() {
        let result = recommendation_file_ids(Err(anyhow::anyhow!("injected lookup failure")));
        assert!(result.is_err());
    }
}
