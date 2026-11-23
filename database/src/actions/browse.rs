//! Adapt UUID-backed identities to the collection API's existing integer handles.

use std::collections::BTreeSet;

use anyhow::Result;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, ExprTrait, FromQueryResult, JoinType,
    sea_query::{
        Alias, Expr, Func, IntoTableRef, Order, Query, SelectStatement, TableRef, UnionType,
        extension::sqlite::SqliteBinOper,
    },
};

use crate::entities::{
    albums, artists, media_file_albums, media_file_artists,
    music_metadata::{
        album_track_anchors, artist_redirects, artists as catalog_artists, audio, browse_albums,
        browse_artists, browse_handles, credits, current_album_titles, discs, edition_matches,
    },
};

use super::{
    collection::CollectionQueryListMode,
    resolved_metadata::{column, like_pattern, nocase, query_models},
};

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Album,
    Artist,
}

impl Kind {
    fn table(self) -> TableRef {
        match self {
            Self::Album => albums::Entity.into_table_ref(),
            Self::Artist => artists::Entity.into_table_ref(),
        }
    }

    fn view(self) -> TableRef {
        match self {
            Self::Album => browse_albums::Entity.into_table_ref(),
            Self::Artist => browse_artists::Entity.into_table_ref(),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Album => "album",
            Self::Artist => "artist",
        }
    }

    fn identities(self) -> SelectStatement {
        let mut query = Query::select();
        match self {
            Self::Album => {
                use current_album_titles::Column as T;
                query
                    .column(T::Id)
                    .expr_as(column(T::Title), "name")
                    .from(current_album_titles::Entity);
            }
            Self::Artist => {
                use artist_redirects::Column as R;
                use catalog_artists::Column as A;
                let target = Alias::new("canonical_artist");
                query
                    .column(A::Id.as_column_ref())
                    .expr_as(
                        Func::coalesce([Expr::col((target.clone(), A::Name)), column(A::Name)]),
                        "name",
                    )
                    .from(catalog_artists::Entity)
                    .left_join(
                        artist_redirects::Entity,
                        column(R::SourceUuid).eq(column(A::Id)),
                    )
                    .join_as(
                        JoinType::LeftJoin,
                        catalog_artists::Entity,
                        target.clone(),
                        Expr::col((target, A::Id)).eq(column(R::TargetUuid)),
                    );
            }
        }
        query
    }

    pub(super) fn old_operator(self) -> &'static str {
        match self {
            Self::Album => "lib::album",
            Self::Artist => "lib::artist",
        }
    }

    fn uuid_operator(self) -> &'static str {
        match self {
            Self::Album => "lib::release",
            Self::Artist => "lib::artist_id",
        }
    }
}

fn alias_col(table: &str, column: &str) -> Expr {
    Expr::col((Alias::new(table), Alias::new(column)))
}

fn browse_rows(kind: Kind) -> SelectStatement {
    // Reuse the legacy alphabetic bucket if that spelling has already been indexed.
    let name = alias_col("browse", "name");
    let first = Func::cust("substr").args([name.clone(), Expr::val(1), Expr::val(1)]);
    let capital: Expr = Func::upper(first).into();
    let bucket = Expr::case(
        capital
            .clone()
            .binary(SqliteBinOper::Glob, Expr::val("[A-Z]")),
        capital,
    )
    .finally("#");
    let mut query = Query::select();
    query
        .columns(["id", "name", "source_uuid"].map(|c| (Alias::new("browse"), Alias::new(c))))
        .expr_as(
            Func::coalesce([alias_col("old", "group"), bucket.into()]),
            "group",
        )
        .from_as(kind.view(), "browse")
        .join_as(
            JoinType::LeftJoin,
            kind.table(),
            "old",
            alias_col("old", "name").eq(name),
        );
    query
}

pub(super) async fn portable_clause(
    db: &impl ConnectionTrait,
    operator_name: String,
    parameter: String,
) -> Result<(String, String)> {
    let kind = match operator_name.as_str() {
        "lib::album" => Kind::Album,
        "lib::artist" => Kind::Artist,
        _ => return Ok((operator_name, parameter)),
    };
    if let Ok(id) = parameter.parse::<i32>()
        && id < 0
    {
        use browse_handles::Column as H;
        let row = db
            .query_one(
                Query::select()
                    .column(H::SourceUuid)
                    .from(browse_handles::Entity)
                    .and_where(H::Kind.eq(kind.label()))
                    .and_where(H::Id.eq(-i64::from(id))),
            )
            .await?
            .ok_or(crate::error::ExpectedError::NotFound { what: "collection" })?;
        let source: String = row.try_get("", "source_uuid")?;
        let parameter = match kind {
            Kind::Album => super::resolved_metadata::edits::portable_edition(db, source.parse()?)
                .await?
                .to_string(),
            Kind::Artist => source,
        };
        return Ok((kind.uuid_operator().into(), parameter));
    }
    Ok((operator_name, parameter))
}

pub(super) fn edition_ids(id: &str) -> SelectStatement {
    use album_track_anchors::Column as A;
    use edition_matches::Column as M;
    let anchored = Query::select()
        .column(A::AlbumId)
        .from(album_track_anchors::Entity)
        .and_where(A::TrackUuid.eq(id))
        .to_owned();
    Query::select()
        .column(M::AlbumId)
        .from(edition_matches::Entity)
        .and_where(M::EditionUuid.eq(id))
        .union(UnionType::All, anchored)
        .to_owned()
}

pub(super) async fn editable_clause(
    db: &impl ConnectionTrait,
    view: &mut super::mixes::MixQueryView,
) -> Result<()> {
    let kind = match view.operator.as_str() {
        "lib::release" => Kind::Album,
        "lib::artist_id" => Kind::Artist,
        _ => return Ok(()),
    };
    // The existing editor uses integer handles. Keep the stored clause portable across devices.
    use browse_handles::Column as H;
    let takes = match kind {
        Kind::Album => column(H::SourceUuid).in_subquery(edition_ids(&view.parameter)),
        Kind::Artist => H::SourceUuid.eq(&view.parameter),
    };
    let present = Query::select()
        .column((Alias::new("edition"), Alias::new("id")))
        .from_subquery(kind.identities(), "edition")
        .and_where(alias_col("edition", "id").eq(column(H::SourceUuid)))
        .to_owned();
    let row = db
        .query_one(
            Query::select()
                .expr_as(Expr::val(0).sub(column(H::Id)), "id")
                .from(browse_handles::Entity)
                .and_where(H::Kind.eq(kind.label()))
                .and_where(takes)
                .and_where(Expr::exists(present)),
        )
        .await?;
    if let Some(row) = row {
        view.operator = kind.old_operator().into();
        view.parameter = row.try_get::<i32>("", "id")?.to_string();
    } else {
        view.available = false;
    }
    Ok(())
}

pub(super) async fn counts(db: &DatabaseConnection, kind: Kind) -> Result<Vec<(String, i32)>> {
    if matches!(kind, Kind::Album) {
        super::resolved_metadata::ensure_tracks_current(db).await?;
    }
    let group = Alias::new("group");
    let rows = db
        .query_all(
            Query::select()
                .column(group.clone())
                .expr_as(Func::count(Expr::val(1)), "n")
                .from_subquery(browse_rows(kind), "items")
                .group_by_col(group.clone())
                .order_by(group, Order::Asc),
        )
        .await?;
    rows.iter()
        .map(|r| Ok((r.try_get("", "group")?, r.try_get("", "n")?)))
        .collect()
}

pub(super) async fn groups<T: FromQueryResult>(
    db: &DatabaseConnection,
    kind: Kind,
    groups: &[String],
) -> Result<Vec<T>> {
    if matches!(kind, Kind::Album) {
        super::resolved_metadata::ensure_tracks_current(db).await?;
    }
    if groups.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = Query::select();
    query
        .columns(["id", "name", "source_uuid", "group"].map(Alias::new))
        .from_subquery(browse_rows(kind), "items")
        .and_where(Expr::col(Alias::new("group")).is_in(groups.iter().map(String::as_str)))
        .order_by_expr(nocase(Expr::col(Alias::new("name"))), Order::Asc)
        .order_by(Alias::new("id"), Order::Asc);
    query_models(db, &query).await
}

pub(super) async fn by_id<T: FromQueryResult>(
    db: &DatabaseConnection,
    kind: Kind,
    ids: &[i32],
) -> Result<Vec<T>> {
    use browse_handles::Column as H;
    let mut found = Vec::new();
    // Positive IDs from previously saved navigation remain resolvable after the browse switch.
    for ids in ids.chunks(400) {
        let names = ["id", "name", "source_uuid", "group"].map(Alias::new);
        let old = Query::select()
            .columns(names)
            .from(kind.table())
            .and_where(Expr::col(Alias::new("id")).is_in(ids.iter().copied()))
            .to_owned();
        let items = Query::select()
            .expr_as(Expr::val(0).sub(column(H::Id)), "id")
            .column((Alias::new("entity"), Alias::new("name")))
            .expr_as(alias_col("entity", "id"), "source_uuid")
            .expr_as(
                Func::coalesce([alias_col("old", "group"), Expr::val("#")]),
                "group",
            )
            .from(browse_handles::Entity)
            .join_subquery(
                JoinType::InnerJoin,
                kind.identities(),
                "entity",
                alias_col("entity", "id").eq(column(H::SourceUuid)),
            )
            .join_as(
                JoinType::LeftJoin,
                kind.table(),
                "old",
                alias_col("old", "name").eq(alias_col("entity", "name")),
            )
            .and_where(H::Kind.eq(kind.label()))
            .and_where(H::Id.is_in(ids.iter().map(|&id| -i64::from(id))))
            .union(UnionType::All, old)
            .to_owned();
        found.extend(query_models::<T>(db, &items).await?);
    }
    Ok(found)
}

pub(super) async fn list<T: FromQueryResult>(
    db: &DatabaseConnection,
    kind: Kind,
    limit: u64,
    mode: CollectionQueryListMode,
) -> Result<Vec<T>> {
    if matches!(kind, Kind::Album) {
        super::resolved_metadata::ensure_tracks_current(db).await?;
    }
    let limit = u64::try_from(i64::try_from(limit)?)?;
    let mut query = Query::select();
    query
        .columns(["id", "name", "source_uuid", "group"].map(Alias::new))
        .from_subquery(browse_rows(kind), "items")
        .limit(limit);

    let id = Expr::col(Alias::new("id"));
    match mode {
        CollectionQueryListMode::Forward => query.order_by_expr(Func::abs(id).into(), Order::Asc),
        CollectionQueryListMode::Reverse => query.order_by_expr(Func::abs(id).into(), Order::Desc),
        CollectionQueryListMode::Random => query.order_by_expr(Func::random().into(), Order::Asc),
        CollectionQueryListMode::Name => query
            .order_by_expr(nocase(Expr::col(Alias::new("name"))), Order::Asc)
            .order_by(Alias::new("id"), Order::Asc),
    };
    query_models(db, &query).await
}

pub(super) async fn resolve_search(
    db: &DatabaseConnection,
    kind: Kind,
    query: &str,
    legacy: &[i64],
    limit: usize,
) -> Result<Vec<i64>> {
    if matches!(kind, Kind::Album) {
        super::resolved_metadata::ensure_tracks_current(db).await?;
    }
    let pattern = query
        .trim_end_matches('*')
        .split_whitespace()
        .map(|term| {
            term.replace('!', "!!")
                .replace('%', "!%")
                .replace('_', "!_")
        })
        .collect::<Vec<_>>()
        .join("%");
    let pattern = format!("%{pattern}%");

    // Match the current metadata and former artist names alongside the legacy FTS results.
    let mut matches = alias_col("browse", "name").like(like_pattern(&pattern));
    if matches!(kind, Kind::Artist) {
        use artist_redirects::Column as R;
        use catalog_artists::Column as A;
        let former = Query::select()
            .column(R::SourceUuid.as_column_ref())
            .from(artist_redirects::Entity)
            .inner_join(
                catalog_artists::Entity,
                column(A::Id).eq(column(R::SourceUuid)),
            )
            .and_where(column(R::TargetUuid).eq(alias_col("browse", "source_uuid")))
            .and_where(column(A::Name).like(like_pattern(&pattern)))
            .to_owned();
        matches = matches.or(Expr::exists(former));
    }
    let rows = db
        .query_all(
            Query::select()
                .column((Alias::new("browse"), Alias::new("id")))
                .from_as(kind.view(), "browse")
                .and_where(matches)
                .order_by(Alias::new("name"), Order::Asc)
                .order_by(Alias::new("id"), Order::Asc)
                .limit(u64::try_from(i64::try_from(limit)?)?),
        )
        .await?;
    let mut ids = rows
        .iter()
        .map(|row| row.try_get::<i64>("", "id"))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut seen: BTreeSet<_> = ids.iter().copied().collect();

    // Preserve FTS matches on transliterations and legacy names after switching to UUID handles.
    for old in legacy.chunks(400) {
        let rows = db
            .query_all(
                Query::select()
                    .column(Alias::new("id"))
                    .from(kind.view())
                    .and_where(Expr::col(Alias::new("id")).is_in(old.iter().copied()))
                    .union(UnionType::Distinct, legacy_matches(kind, old)),
            )
            .await?;
        for row in rows {
            let id = row.try_get("", "id")?;
            if ids.len() < limit && seen.insert(id) {
                ids.push(id);
            }
        }
    }
    Ok(ids)
}

fn legacy_matches(kind: Kind, ids: &[i64]) -> SelectStatement {
    let mut query = Query::select();
    match kind {
        Kind::Album => {
            use albums::Column as A;
            use browse_albums::Column as B;
            use media_file_albums::Column as L;
            query
                .column(B::Id.as_column_ref())
                .from(albums::Entity)
                .inner_join(
                    media_file_albums::Entity,
                    column(L::AlbumUuid).eq(column(A::SourceUuid)),
                )
                .inner_join(
                    audio::Entity,
                    column(audio::Column::Id).eq(column(L::TrackUuid)),
                )
                .inner_join(
                    discs::Entity,
                    column(discs::Column::Id).eq(column(audio::Column::DiscId)),
                )
                .inner_join(
                    browse_albums::Entity,
                    column(B::SourceUuid).eq(column(discs::Column::AlbumId)),
                )
                .and_where(A::Id.is_in(ids.iter().copied()));
        }
        Kind::Artist => {
            use artist_redirects::Column as R;
            use artists::Column as A;
            use browse_artists::Column as B;
            use credits::Column as C;
            use media_file_artists::Column as L;
            let legacy = Alias::new("legacy_redirect");
            let canonical: Expr = Func::coalesce([
                column(R::TargetUuid),
                column(C::ArtistId),
                Expr::col((legacy.clone(), R::TargetUuid)),
                column(A::SourceUuid),
            ])
            .into();

            // Each confirmed credit can resolve to its own artist, even for a shared legacy name.
            query
                .column(B::Id.as_column_ref())
                .from(artists::Entity)
                .join_as(
                    JoinType::LeftJoin,
                    artist_redirects::Entity,
                    legacy.clone(),
                    Expr::col((legacy, R::SourceUuid)).eq(column(A::SourceUuid)),
                )
                .left_join(
                    media_file_artists::Entity,
                    column(L::ArtistUuid).eq(column(A::SourceUuid)),
                )
                .left_join(
                    audio::Entity,
                    column(audio::Column::Id).eq(column(L::TrackUuid)),
                )
                .left_join(
                    credits::Entity,
                    column(C::FileId)
                        .eq(column(audio::Column::FileId))
                        .and(C::Scope.eq(2))
                        .and(C::Role.eq("Primary Artist"))
                        .and(column(C::CreditedAs).eq(nocase(column(A::Name)))),
                )
                .left_join(
                    artist_redirects::Entity,
                    column(R::SourceUuid).eq(column(C::ArtistId)),
                )
                .inner_join(browse_artists::Entity, column(B::SourceUuid).eq(canonical))
                .and_where(A::Id.is_in(ids.iter().copied()));
        }
    }
    query
}

macro_rules! resolved_browse_query {
    ($model:ty, $kind:ident) => {
        #[async_trait::async_trait]
        impl super::collection::CollectionQuery for $model {
            fn collection_type() -> super::collection::CollectionQueryType {
                super::collection::CollectionQueryType::$kind
            }

            async fn query_builder(
                _db: &crate::connection::MainDbConnection,
                id: i32,
            ) -> anyhow::Result<Vec<(String, String)>> {
                Ok(vec![(
                    super::browse::Kind::$kind.old_operator().into(),
                    id.to_string(),
                )])
            }

            async fn count_by_first_letter(
                db: &crate::connection::MainDbConnection,
            ) -> anyhow::Result<Vec<(String, i32)>> {
                super::browse::counts(db, super::browse::Kind::$kind).await
            }

            async fn get_groups(
                db: &crate::connection::MainDbConnection,
                groups: Vec<String>,
            ) -> anyhow::Result<Vec<(String, Vec<(Self, std::collections::HashSet<i32>)>)>> {
                let items: Vec<Self> =
                    super::browse::groups(db, super::browse::Kind::$kind, &groups).await?;
                let mut buckets = std::collections::BTreeMap::<_, Vec<_>>::new();
                for item in items {
                    buckets
                        .entry(item.group.clone())
                        .or_default()
                        .push((item, Default::default()));
                }
                Ok(groups
                    .into_iter()
                    .map(|group| {
                        let items = buckets.remove(&group).unwrap_or_default();
                        (group, items)
                    })
                    .collect())
            }

            async fn get_by_ids(
                db: &crate::connection::MainDbConnection,
                ids: &[i32],
            ) -> anyhow::Result<Vec<Self>> {
                super::browse::by_id(db, super::browse::Kind::$kind, ids).await
            }

            async fn list(
                db: &crate::connection::MainDbConnection,
                limit: u64,
                mode: super::collection::CollectionQueryListMode,
            ) -> anyhow::Result<Vec<Self>> {
                super::browse::list(db, super::browse::Kind::$kind, limit, mode).await
            }

            fn id(&self) -> i32 {
                self.id
            }
            fn name(&self) -> &str {
                &self.name
            }
            fn readonly(&self) -> bool {
                false
            }
        }
    };
}

pub(super) use resolved_browse_query;
