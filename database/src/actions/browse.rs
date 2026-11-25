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
    audio_artists,
    music_metadata::{
        albums as editions, artists as catalog_artists, audio, browse_handles, credits, discs,
        release_identity,
    },
    portable_albums, portable_artists, track_album_sources,
};

use super::{
    collection::CollectionQueryListMode,
    resolved_metadata::{
        column, like_pattern, nocase,
        queries::{field, subquery},
        query_models, scalar,
        summary::{TITLES, titles},
    },
};

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Album,
    Artist,
}

impl Kind {
    fn table(self) -> TableRef {
        match self {
            Self::Album => portable_albums::Entity.into_table_ref(),
            Self::Artist => portable_artists::Entity.into_table_ref(),
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Album => "album",
            Self::Artist => "artist",
        }
    }

    fn identities(self) -> SelectStatement {
        match self {
            Self::Album => Query::select()
                .column(Alias::new("id"))
                .expr_as(field(TITLES, "title"), "name")
                .from(titles())
                .to_owned(),
            Self::Artist => Query::select()
                .expr_as(column(catalog_artists::Column::Id), "id")
                .column(catalog_artists::Column::Name)
                .from(catalog_artists::Entity)
                .union(
                    UnionType::All,
                    Query::select()
                        .columns([
                            portable_artists::Column::SourceUuid,
                            portable_artists::Column::Name,
                        ])
                        .from(portable_artists::Entity)
                        .to_owned(),
                )
                .to_owned(),
        }
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

fn browse_group(kind: Kind, id: Expr, name: Expr) -> Expr {
    // Mapped identities retain their saved bucket even when their title changes.
    let mut saved = Query::select();
    match kind {
        Kind::Album => saved
            .column(portable_albums::Column::Group)
            .and_where(column(portable_albums::Column::SourceUuid).eq(id.clone())),
        Kind::Artist => saved
            .column(portable_artists::Column::Group)
            .and_where(column(portable_artists::Column::SourceUuid).eq(id.clone())),
    };
    let saved = saved.from(kind.table()).to_owned();
    let mut buckets = vec![scalar(saved)];
    if matches!(kind, Kind::Artist) {
        let identified = Query::select()
            .column(catalog_artists::Column::Group)
            .from(catalog_artists::Entity)
            .and_where(column(catalog_artists::Column::Id).eq(id.clone()))
            .to_owned();
        buckets.push(scalar(identified));
    }

    // Edition IDs differ from the old album UUID. Follow the track membership.
    if matches!(kind, Kind::Album) {
        use portable_albums::Column as A;
        use track_album_sources::Column as L;
        let edition = Query::select()
            .column(A::Group.as_column_ref())
            .from(discs::Entity)
            .inner_join(
                audio::Entity,
                column(audio::Column::DiscId).eq(column(discs::Column::Id)),
            )
            .inner_join(
                track_album_sources::Entity,
                column(L::TrackUuid).eq(column(audio::Column::Id)),
            )
            .inner_join(
                portable_albums::Entity,
                column(A::SourceUuid).eq(column(L::AlbumUuid)),
            )
            .and_where(column(discs::Column::AlbumId).eq(id))
            .order_by(A::Id, Order::Asc)
            .limit(1)
            .to_owned();
        buckets.push(scalar(edition));
    }

    // Only identities without an indexed bucket need a display-name fallback.
    let first = Func::cust("substr").args([name, Expr::val(1), Expr::val(1)]);
    let capital: Expr = Func::upper(first).into();
    let bucket = Expr::case(
        capital
            .clone()
            .binary(SqliteBinOper::Glob, Expr::val("[A-Z]")),
        capital,
    )
    .finally("#");
    buckets.push(bucket.into());
    Func::coalesce(buckets).into()
}

fn browse_selection(kind: Kind) -> TableRef {
    let rows = match kind {
        Kind::Album => album_rows(),
        Kind::Artist => artist_rows(),
    };
    subquery(rows, "browse")
}

fn album_rows() -> SelectStatement {
    use browse_handles::Column as H;
    use track_album_sources::Column as L;
    let assigned = Query::select()
        .column(L::TrackUuid.as_column_ref())
        .from(track_album_sources::Entity)
        .inner_join(
            audio::Entity,
            column(audio::Column::Id).eq(column(L::TrackUuid)),
        )
        .and_where(column(L::AlbumUuid).eq(field(TITLES, "id")))
        .to_owned();
    let portable = field(TITLES, "portable_title").is_not_null();
    let id: Expr = Expr::case(portable.clone(), field(TITLES, "legacy_id"))
        .finally(Expr::val(0).sub(column(H::Id)))
        .into();
    // Show populated editions and preserve empty portable albums with saved IDs.
    Query::select()
        .expr_as(id, "id")
        .expr_as(field(TITLES, "title"), "name")
        .expr_as(field(TITLES, "id"), "source_uuid")
        .from(titles())
        .left_join(
            browse_handles::Entity,
            H::Kind
                .eq(Kind::Album.label())
                .and(column(H::SourceUuid).eq(field(TITLES, "id"))),
        )
        .and_where(
            field(TITLES, "track_count")
                .gt(0)
                .or(portable.and(Expr::not_exists(assigned))),
        )
        .and_where(
            field(TITLES, "portable_title")
                .is_not_null()
                .or(H::Id.is_not_null()),
        )
        .to_owned()
}

fn artist_rows() -> SelectStatement {
    use audio_artists::Column as L;
    use browse_handles::Column as H;
    use credits::Column as C;
    use portable_artists::Column as A;
    let known = Query::select()
        .column(catalog_artists::Column::Id)
        .from_as(catalog_artists::Entity, "known")
        .and_where(field("known", "musicbrainz_id").is_not_null())
        .and_where(field("known", "name").eq(nocase(column(A::Name))))
        .to_owned();
    let credit = Query::select()
        .column(C::FileId)
        .from(credits::Entity)
        .and_where(column(C::FileId).eq(column(audio::Column::FileId)))
        .and_where(C::Scope.eq(2))
        .and_where(C::Role.eq("Primary Artist"))
        .and_where(C::ArtistId.is_not_null())
        .and_where(column(C::CreditedAs).eq(nocase(column(A::Name))))
        .to_owned();
    let unresolved = Query::select()
        .column(L::TrackUuid.as_column_ref())
        .from(audio_artists::Entity)
        .left_join(
            audio::Entity,
            column(audio::Column::Id).eq(column(L::TrackUuid)),
        )
        .and_where(column(L::ArtistUuid).eq(column(A::SourceUuid)))
        .and_where(Expr::not_exists(credit))
        .to_owned();

    // Portable artist rows keep their original numeric IDs for saved navigation.
    let mut portable = Query::select()
        .columns([A::Id, A::Name, A::SourceUuid].map(|c| c.as_column_ref()))
        .from(portable_artists::Entity)
        .and_where(Expr::not_exists(known).or(Expr::exists(unresolved)))
        .to_owned();
    use catalog_artists::Column as I;
    let identified = Query::select()
        .expr_as(Expr::val(0).sub(column(H::Id)), "id")
        .column(I::Name.as_column_ref())
        .expr_as(column(I::Id), "source_uuid")
        .from(catalog_artists::Entity)
        .inner_join(
            browse_handles::Entity,
            H::Kind
                .eq(Kind::Artist.label())
                .and(column(H::SourceUuid).eq(column(I::Id))),
        )
        .to_owned();
    portable.union(UnionType::All, identified).to_owned()
}

fn browse_rows(kind: Kind) -> SelectStatement {
    let mut query = Query::select();
    query
        .columns(["id", "name", "source_uuid"].map(|c| (Alias::new("browse"), Alias::new(c))))
        .expr_as(
            browse_group(
                kind,
                alias_col("browse", "source_uuid"),
                alias_col("browse", "name"),
            ),
            "group",
        )
        .from(browse_selection(kind));
    query
}

async fn collection_models<T: FromQueryResult>(
    db: &DatabaseConnection,
    kind: Kind,
    query: SelectStatement,
) -> Result<Vec<T>> {
    if matches!(kind, Kind::Album) {
        // Decode public handles using the album entity's physical column names.
        let rows = Query::select()
            .expr_as(alias_col("items", "id"), "legacy_id")
            .expr_as(alias_col("items", "name"), "portable_title")
            .expr_as(alias_col("items", "source_uuid"), "id")
            .expr_as(alias_col("items", "group"), "portable_group")
            .from_subquery(query, "items")
            .to_owned();
        return query_models(db, &rows).await;
    }
    query_models(db, &query).await
}

pub(super) async fn album_for_file(
    db: &DatabaseConnection,
    track_uuid: &str,
) -> Result<Option<portable_albums::Model>> {
    super::resolved_metadata::ensure_tracks_current(db).await?;
    let edition = Query::select()
        .column(discs::Column::AlbumId.as_column_ref())
        .from(audio::Entity)
        .inner_join(
            discs::Entity,
            column(audio::Column::DiscId).eq(column(discs::Column::Id)),
        )
        .and_where(audio::Column::Id.eq(track_uuid))
        .to_owned();

    // Resolve the actual edition of this audio file, including its current title.
    let query = Query::select()
        .columns(["id", "name", "source_uuid", "group"].map(Alias::new))
        .from_subquery(browse_rows(Kind::Album), "edition")
        .and_where(Expr::col(Alias::new("source_uuid")).in_subquery(edition))
        .limit(1)
        .to_owned();
    let rows: Vec<portable_albums::Model> = collection_models(db, Kind::Album, query).await?;
    Ok(rows.into_iter().next())
}

/// The albums the collection lists for these editions: the id and the title it gives each,
/// with the edition's own id as `source_uuid`.
pub(super) fn edition_albums(editions: SelectStatement) -> SelectStatement {
    Query::select()
        .columns(["id", "name", "source_uuid"].map(|c| (Alias::new("browse"), Alias::new(c))))
        .from(browse_selection(Kind::Album))
        .and_where(alias_col("browse", "source_uuid").in_subquery(editions))
        .to_owned()
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
            Kind::Album => portable_edition(db, &source).await?,
            Kind::Artist => source,
        };
        return Ok((kind.uuid_operator().into(), parameter));
    }
    Ok((operator_name, parameter))
}

pub(super) fn edition_ids(id: &str) -> SelectStatement {
    use editions::Column as A;
    use portable_albums::Column as P;
    use release_identity::Column as R;
    let mut local = Query::select()
        .column(A::Id)
        .from(editions::Entity)
        .and_where(A::Id.eq(id))
        .union(
            UnionType::Distinct,
            Query::select()
                .column(P::SourceUuid)
                .from(portable_albums::Entity)
                .and_where(P::SourceUuid.eq(id))
                .to_owned(),
        )
        .to_owned();
    // An exact UUID takes precedence over evidence linking a portable edition ID.
    let identified = Query::select()
        .expr(Func::min(column(R::AlbumId)))
        .from(release_identity::Entity)
        .and_where(R::EditionUuid.eq(id))
        .and_where(Expr::not_exists(local.clone()))
        .and_having(Expr::from(Func::count(Expr::val(1))).eq(1))
        .to_owned();
    let anchored = Query::select()
        .column(discs::Column::AlbumId.as_column_ref())
        .from(audio::Entity)
        .inner_join(
            discs::Entity,
            column(discs::Column::Id).eq(column(audio::Column::DiscId)),
        )
        .and_where(audio::Column::Id.eq(id))
        .to_owned();
    local
        .union(UnionType::All, identified)
        .union(UnionType::All, anchored)
        .to_owned()
}

async fn portable_edition(db: &impl ConnectionTrait, release: &str) -> Result<String> {
    use release_identity::Column as R;
    let duplicate = Query::select()
        .column(Alias::new("album_id"))
        .from_as(release_identity::Entity, "other")
        .and_where(field("other", "edition_uuid").eq(column(R::EditionUuid)))
        .and_where(field("other", "album_id").ne(release))
        .to_owned();
    let shared = Query::select()
        .column(R::EditionUuid)
        .from(release_identity::Entity)
        .and_where(R::AlbumId.eq(release))
        .and_where(Expr::not_exists(duplicate))
        .to_owned();
    // A synchronized track is a portable fallback when the edition has no unique release ID.
    let anchor = Query::select()
        .column(audio::Column::Id.as_column_ref())
        .from(discs::Entity)
        .inner_join(
            audio::Entity,
            column(audio::Column::DiscId).eq(column(discs::Column::Id)),
        )
        .and_where(discs::Column::AlbumId.eq(release))
        .order_by(audio::Column::Id.as_column_ref(), Order::Asc)
        .limit(1)
        .to_owned();
    let row = db
        .query_one(Query::select().expr_as(
            Func::coalesce([scalar(shared), scalar(anchor), Expr::val(release)]),
            "edition_uuid",
        ))
        .await?;
    Ok(row
        .map(|row| row.try_get("", "edition_uuid"))
        .transpose()?
        .unwrap_or_else(|| release.to_owned()))
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
    collection_models(db, kind, query).await
}

pub(super) async fn by_id<T: FromQueryResult>(
    db: &DatabaseConnection,
    kind: Kind,
    ids: &[i32],
) -> Result<Vec<T>> {
    if matches!(kind, Kind::Album) {
        super::resolved_metadata::ensure_tracks_current(db).await?;
    }
    use browse_handles::Column as H;
    let mut found = Vec::new();
    // Positive IDs from previously saved navigation remain resolvable after the browse switch.
    for ids in ids.chunks(400) {
        let mut saved = Query::select();
        match kind {
            Kind::Album => {
                use portable_albums::Column as A;
                saved
                    .expr_as(column(A::Id), "id")
                    .expr_as(field(TITLES, "title"), "name")
                    .expr_as(column(A::SourceUuid), "source_uuid")
                    .expr_as(column(A::Group), "group")
                    .inner_join(titles(), column(A::SourceUuid).eq(field(TITLES, "id")))
                    .and_where(A::Id.is_in(ids.iter().copied()));
            }
            Kind::Artist => {
                saved
                    .columns(["id", "name", "source_uuid", "group"].map(Alias::new))
                    .and_where(portable_artists::Column::Id.is_in(ids.iter().copied()));
            }
        }
        let saved = saved.from(kind.table()).to_owned();
        let items = Query::select()
            .expr_as(Expr::val(0).sub(column(H::Id)), "id")
            .column((Alias::new("entity"), Alias::new("name")))
            .expr_as(alias_col("entity", "id"), "source_uuid")
            .expr_as(
                browse_group(kind, alias_col("entity", "id"), alias_col("entity", "name")),
                "group",
            )
            .from(browse_handles::Entity)
            .join_subquery(
                JoinType::InnerJoin,
                kind.identities(),
                "entity",
                alias_col("entity", "id").eq(column(H::SourceUuid)),
            )
            .and_where(H::Kind.eq(kind.label()))
            .and_where(H::Id.is_in(ids.iter().map(|&id| -i64::from(id))))
            .union(UnionType::All, saved)
            .to_owned();
        found.extend(collection_models::<T>(db, kind, items).await?);
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
    collection_models(db, kind, query).await
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

    let matches = alias_col("browse", "name").like(like_pattern(&pattern));
    let rows = db
        .query_all(
            Query::select()
                .column((Alias::new("browse"), Alias::new("id")))
                .from(browse_selection(kind))
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
                    .from(browse_selection(kind))
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
            use portable_albums::Column as A;
            use track_album_sources::Column as L;
            query
                .column((Alias::new("browse"), Alias::new("id")))
                .from(portable_albums::Entity)
                .inner_join(
                    track_album_sources::Entity,
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
                    browse_selection(Kind::Album),
                    field("browse", "source_uuid").eq(column(discs::Column::AlbumId)),
                )
                .and_where(A::Id.is_in(ids.iter().copied()));
        }
        Kind::Artist => {
            use audio_artists::Column as L;
            use credits::Column as C;
            use portable_artists::Column as A;
            let identity = Func::coalesce([column(C::ArtistId), column(A::SourceUuid)]);
            // File evidence can resolve a spelling to a known artist identity.
            query
                .column((Alias::new("browse"), Alias::new("id")))
                .from(portable_artists::Entity)
                .left_join(
                    audio_artists::Entity,
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
                .inner_join(
                    browse_selection(Kind::Artist),
                    field("browse", "source_uuid").eq(identity),
                )
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
