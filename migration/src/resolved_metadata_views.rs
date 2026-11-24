//! Read queries exposed to the catalog entities under stable SQLite view names.

use sea_orm_migration::{
    prelude::*,
    sea_orm::ConnectionTrait,
    sea_query::{
        Asterisk, BinOper, CommonTableExpression, SelectStatement, SqliteQueryBuilder, UnionType,
        WindowStatement, WithClause,
    },
};

type View = (&'static str, fn() -> SelectStatement);

const VIEWS: &[View] = &[
    ("current_recording_audio", current_recording_audio),
    ("recording_evidence", recording_evidence),
    ("recording_aliases", recording_aliases),
    ("track_recordings", track_recordings),
    ("album_effective_fields", album_effective_fields),
    ("live_summaries", live_summaries),
    ("edition_matches", edition_matches),
    ("synced_album_groups", synced_album_groups),
    ("artist_redirects", artist_redirects),
    ("artist_identities", artist_identities),
    ("current_album_titles", current_album_titles),
    ("browse_albums", browse_albums),
    ("browse_artists", browse_artists),
];

pub(super) async fn install(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    // SeaQuery builds each SELECT. It has no CREATE VIEW statement builder.
    for (name, query) in VIEWS {
        let query = query().to_string(SqliteQueryBuilder);
        manager
            .get_connection()
            .execute_unprepared(&format!("CREATE VIEW music_metadata_{name} AS {query}"))
            .await?;
    }
    Ok(())
}

pub(super) async fn uninstall(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    // SeaQuery has no DROP VIEW statement builder.
    for (name, _) in VIEWS.iter().rev() {
        manager
            .get_connection()
            .execute_unprepared(&format!("DROP VIEW music_metadata_{name}"))
            .await?;
    }
    Ok(())
}

fn alias(name: &str) -> Alias {
    Alias::new(name)
}

fn catalog(name: &str) -> Alias {
    Alias::new(format!("music_metadata_{name}"))
}

fn column(table: &str, column: &str) -> Expr {
    Expr::col((alias(table), alias(column)))
}

fn scalar(query: SelectStatement) -> Expr {
    Expr::SubQuery(None, Box::new(query.into()))
}

fn nocase(expr: Expr) -> Expr {
    expr.binary(BinOper::Custom("COLLATE"), Expr::custom_keyword("NOCASE"))
}

fn exists_in(table: Alias, condition: Expr) -> SelectStatement {
    Query::select()
        .expr(Expr::val(1))
        .from(table)
        .and_where(condition)
        .to_owned()
}

fn current_recording_audio() -> SelectStatement {
    // SQLite IS compares nullable content versions without excluding NULL pairs.
    Query::select()
        .column((alias("audio"), Asterisk))
        .from_as(catalog("audio"), alias("audio"))
        .join_as(
            JoinType::InnerJoin,
            alias("media_files"),
            alias("file"),
            column("audio", "file_id").eq(column("file", "id")),
        )
        .and_where(
            column("audio", "recording_content_hash")
                .binary(BinOper::Custom("IS"), column("file", "content_hash")),
        )
        .and_where(
            column("audio", "recording_content_size")
                .binary(BinOper::Custom("IS"), column("file", "content_size")),
        )
        .to_owned()
}

fn recording_evidence() -> SelectStatement {
    // The same shape exposes evidence from verified files and external providers.
    let providers = Query::select()
        .exprs([Expr::val(None::<i32>), Expr::val(None::<String>)])
        .columns(
            [
                "provider",
                "resource_id",
                "recording_id",
                "recording_mbid",
                "recording_isrc",
            ]
            .map(alias),
        )
        .from(catalog("recording_sources"))
        .to_owned();
    Query::select()
        .column(alias("file_id"))
        .expr_as(Expr::col(alias("id")), alias("audio_id"))
        .expr_as(Expr::val(None::<String>), alias("provider"))
        .expr_as(Expr::val(None::<String>), alias("resource_id"))
        .column(alias("recording_id"))
        .expr_as(Expr::col(alias("recording_mbid")), alias("mbid"))
        .expr_as(Expr::col(alias("recording_isrc")), alias("isrc"))
        .from(catalog("current_recording_audio"))
        .union(UnionType::All, providers)
        .to_owned()
}

fn recording_aliases() -> SelectStatement {
    // Follow an ISRC-only identity only while there is one matching upgrade.
    let conflicting = exists_in(
        catalog("recordings"),
        column("music_metadata_recordings", "isrc")
            .eq(column("current", "isrc"))
            .and(column("music_metadata_recordings", "id").ne(column("current", "id"))),
    );
    let logical_tracks = Query::select()
        .columns([
            (alias("track"), alias("recording_id")),
            (alias("chosen"), alias("recording_id")),
        ])
        .from_as(catalog("release_tracks"), alias("track"))
        .join_as(
            JoinType::InnerJoin,
            catalog("track_recordings"),
            alias("chosen"),
            column("chosen", "track_id").eq(column("track", "id")),
        )
        .and_where(column("chosen", "recording_id").ne(column("track", "recording_id")))
        .to_owned();

    // Logical track recordings can also be superseded by verified file evidence.
    Query::select()
        .expr_as(column("previous", "id"), alias("source_id"))
        .expr_as(column("current", "id"), alias("target_id"))
        .from_as(catalog("recordings"), alias("previous"))
        .join_as(
            JoinType::InnerJoin,
            catalog("recordings"),
            alias("current"),
            column("current", "isrc").eq(column("previous", "legacy_isrc")),
        )
        .and_where(column("previous", "isrc").is_null())
        .and_where(column("current", "id").ne(column("previous", "id")))
        .and_where(Expr::not_exists(conflicting))
        .union(UnionType::All, logical_tracks)
        .to_owned()
}

fn track_recordings() -> SelectStatement {
    // Segments cannot assert the recording identity of an entire audio file.
    let verified = Query::select()
        .expr(Func::min(column("audio", "recording_id")))
        .from_as(catalog("track_assets"), alias("asset"))
        .join_as(
            JoinType::InnerJoin,
            catalog("current_recording_audio"),
            alias("audio"),
            column("audio", "file_id").eq(column("asset", "file_id")),
        )
        .join_as(
            JoinType::InnerJoin,
            catalog("recordings"),
            alias("recording"),
            column("recording", "id").eq(column("audio", "recording_id")),
        )
        .and_where(column("asset", "track_id").eq(column("track", "id")))
        .and_where(column("asset", "start_ms").eq(0))
        .and_where(column("asset", "end_ms").is_null())
        .and_where(
            column("recording", "musicbrainz_id")
                .is_not_null()
                .or(column("recording", "isrc").is_not_null()),
        )
        .and_having(Expr::from(Func::count_distinct(column("audio", "recording_id"))).eq(1))
        .to_owned();

    // Keep the logical identity if evidence is absent or conflicts.
    Query::select()
        .expr_as(column("track", "id"), alias("track_id"))
        .expr_as(
            Func::coalesce([scalar(verified), column("track", "recording_id")]),
            alias("recording_id"),
        )
        .from_as(catalog("release_tracks"), alias("track"))
        .to_owned()
}

fn album_effective_fields() -> SelectStatement {
    // A local overlay can deliberately mask an approved provider value with NULL.
    let same_field = exists_in(
        catalog("local_overlays"),
        column("music_metadata_local_overlays", "album_id")
            .eq(column("provider", "album_id"))
            .and(column("music_metadata_local_overlays", "field").eq(column("provider", "field"))),
    );
    let approved = Query::select()
        .column((alias("provider"), Asterisk))
        .from_as(catalog("album_field_values"), alias("provider"))
        .and_where(column("provider", "decision").eq(1))
        .and_where(Expr::not_exists(same_field))
        .to_owned();

    // Expose both sources through the same field projection.
    Query::select()
        .columns(["album_id", "field"].map(alias))
        .expr_as(Expr::val(""), alias("provider"))
        .expr_as(Expr::val(""), alias("resource"))
        .column(alias("value"))
        .expr_as(Expr::val(1), alias("decision"))
        .from(catalog("local_overlays"))
        .and_where(Expr::col(alias("album_id")).is_not_null())
        .union(UnionType::All, approved)
        .to_owned()
}

fn live_summaries() -> SelectStatement {
    // Only dirty albums need a live summary. A cross join preserves that access order.
    let mut ordered = Query::select();
    ordered
        .column((alias("disc"), alias("album_id")))
        .from_as(catalog("dirty_summaries"), alias("dirty"))
        .join_as(
            JoinType::CrossJoin,
            catalog("discs"),
            alias("disc"),
            Expr::val(true),
        )
        .join_as(
            JoinType::InnerJoin,
            catalog("audio"),
            alias("audio"),
            column("audio", "disc_id").eq(column("disc", "id")),
        )
        .join_as(
            JoinType::InnerJoin,
            catalog("album_evidence"),
            alias("evidence"),
            column("evidence", "file_id").eq(column("audio", "file_id")),
        )
        .and_where(column("disc", "album_id").eq(column("dirty", "album_id")));

    // Select a winner independently for each field in deterministic disc and track order.
    for (field, priority) in [
        ("title", Some("title_priority")),
        ("version", Some("version_priority")),
        ("catalog_number", None),
        ("release_date", None),
        ("release_type", None),
    ] {
        let mut preference = WindowStatement::partition_by((alias("disc"), alias("album_id")));
        if field != "title" {
            preference.order_by_expr(column("evidence", field).is_null(), Order::Asc);
        }
        if let Some(priority) = priority {
            preference.order_by((alias("evidence"), alias(priority)), Order::Asc);
        }

        // Resolve equal-priority evidence by disc, track, then source row.
        preference
            .order_by((alias("disc"), alias("disc_number")), Order::Asc)
            .order_by_expr(column("audio", "track_number").is_null(), Order::Asc)
            .order_by((alias("audio"), alias("track_number")), Order::Asc)
            .order_by((alias("audio"), alias("file_id")), Order::Asc);
        ordered.expr_window_as(
            Func::cust("FIRST_VALUE").arg(column("evidence", field)),
            preference,
            alias(field),
        );
    }

    // The first row already carries the winner of every independent window.
    let mut pick = WindowStatement::partition_by((alias("disc"), alias("album_id")));
    pick.order_by((alias("audio"), alias("file_id")), Order::Asc);
    ordered.expr_window_as(Func::cust("ROW_NUMBER"), pick, alias("pick"));
    let count = Query::select()
        .expr(Func::count(Expr::val(1)))
        .from_as(catalog("release_tracks"), alias("track"))
        .join_as(
            JoinType::InnerJoin,
            catalog("discs"),
            alias("disc"),
            column("disc", "id").eq(column("track", "disc_id")),
        )
        .and_where(column("disc", "album_id").eq(column("ordered", "album_id")))
        .to_owned();

    // Track counts reflect logical tracks, even when an edition has several encodings.
    Query::select()
        .columns(
            [
                "album_id",
                "title",
                "version",
                "catalog_number",
                "release_date",
                "release_type",
            ]
            .map(alias),
        )
        .expr_as(scalar(count), alias("track_count"))
        .from_subquery(ordered, alias("ordered"))
        .and_where(Expr::col(alias("pick")).eq(1))
        .to_owned()
}

fn edition_matches() -> SelectStatement {
    // An edition key maps through confirmed evidence only when unambiguous.
    let identified = Query::select()
        .expr(Func::min(Expr::col(alias("album_id"))))
        .column(alias("edition_uuid"))
        .from(catalog("release_identity"))
        .group_by_col(alias("edition_uuid"))
        .and_having(Expr::from(Func::count(Expr::val(1))).eq(1))
        .to_owned();
    let matching = Query::select()
        .expr(Func::count(Expr::val(1)))
        .from(catalog("release_identity"))
        .and_where(Expr::col(alias("edition_uuid")).eq(column("edit", "owner_uuid")))
        .to_owned();

    // Shared tracks locate peer editions after explicit identities are considered.
    let local = exists_in(
        catalog("albums"),
        Expr::col(alias("id")).eq(column("edit", "owner_uuid")),
    );
    let anchored = Query::select()
        .expr(Func::min(column("anchor", "album_id")))
        .column((alias("edit"), alias("owner_uuid")))
        .from_as(catalog("sync_edits"), alias("edit"))
        .join_as(
            JoinType::InnerJoin,
            catalog("album_track_anchors"),
            alias("anchor"),
            column("anchor", "track_uuid").eq(column("edit", "edition_uuid")),
        )
        .and_where(column("edit", "kind").eq("anchor"))
        .and_where(Expr::not_exists(local))
        .and_where(scalar(matching).ne(1))
        .group_by_col((alias("edit"), alias("owner_uuid")))
        .and_having(Expr::from(Func::count_distinct(column("anchor", "album_id"))).eq(1))
        .to_owned();

    // A local UUID is authoritative for edits made against the local edition.
    Query::select()
        .expr_as(Expr::col(alias("id")), alias("album_id"))
        .expr_as(Expr::col(alias("id")), alias("edition_uuid"))
        .from(catalog("albums"))
        .union(UnionType::Distinct, identified)
        .union(UnionType::Distinct, anchored)
        .to_owned()
}

fn synced_album_groups() -> SelectStatement {
    Query::select()
        .columns([
            (alias("matched"), alias("album_id")),
            (alias("edit"), alias("owner_uuid")),
            (alias("edit"), alias("target_uuid")),
            (alias("edit"), alias("generation")),
        ])
        .from_as(catalog("sync_edits"), alias("edit"))
        .join_as(
            JoinType::InnerJoin,
            catalog("edition_matches"),
            alias("matched"),
            column("matched", "edition_uuid").eq(column("edit", "owner_uuid")),
        )
        .and_where(column("edit", "kind").eq("album"))
        .to_owned()
}

fn artist_redirects() -> SelectStatement {
    // Enumerate each reachable pair once so cycles terminate.
    let next = Query::select()
        .columns([
            (alias("r"), alias("source_uuid")),
            (alias("next"), alias("target_uuid")),
        ])
        .from_as(alias("reachable"), alias("r"))
        .join_as(
            JoinType::InnerJoin,
            catalog("sync_edits"),
            alias("next"),
            column("next", "kind")
                .eq("artist")
                .and(column("next", "owner_uuid").eq(column("r", "target_uuid"))),
        )
        .and_where(column("next", "target_uuid").is_not_null())
        .to_owned();
    let direct = Query::select()
        .columns(["owner_uuid", "target_uuid"].map(alias))
        .from(catalog("sync_edits"))
        .and_where(Expr::col(alias("kind")).eq("artist"))
        .and_where(Expr::col(alias("target_uuid")).is_not_null())
        .union(UnionType::Distinct, next)
        .to_owned();

    // Prefer a terminal redirect. Within a cycle, choose a stable representative.
    let outgoing = exists_in(
        catalog("sync_edits"),
        Expr::col(alias("kind"))
            .eq("artist")
            .and(Expr::col(alias("owner_uuid")).eq(column("r", "target_uuid"))),
    );
    let cycle = Query::select()
        .expr(Expr::val(1))
        .from_as(alias("reachable"), alias("candidate"))
        .and_where(column("candidate", "source_uuid").eq(column("r", "target_uuid")))
        .and_where(column("candidate", "target_uuid").eq(column("r", "target_uuid")))
        .to_owned();
    let target = || column("r", "target_uuid");
    let canonical = Query::select()
        .column((alias("r"), alias("source_uuid")))
        .expr_as(
            Func::coalesce([
                Expr::from(Func::min(Expr::case(Expr::not_exists(outgoing), target()))),
                Expr::from(Func::min(Expr::case(Expr::exists(cycle), target()))),
            ]),
            alias("target_uuid"),
        )
        .from_as(alias("reachable"), alias("r"))
        .group_by_col((alias("r"), alias("source_uuid")))
        .to_owned();

    // Only expose identities whose representative differs from the source.
    let reachable = CommonTableExpression::new()
        .table_name(alias("reachable"))
        .columns(["source_uuid", "target_uuid"].map(alias))
        .query(direct)
        .to_owned();
    Query::select()
        .columns(["source_uuid", "target_uuid"].map(alias))
        .from_subquery(canonical, alias("canonical"))
        .and_where(Expr::col(alias("source_uuid")).ne(Expr::col(alias("target_uuid"))))
        .with_cte(WithClause::new().recursive(true).cte(reachable).to_owned())
        .to_owned()
}

fn artist_identities() -> SelectStatement {
    Query::select()
        .expr_as(column("artist", "id"), alias("source_id"))
        .column((alias("artist"), alias("source_uuid")))
        .expr_as(
            Func::coalesce([column("target", "id"), column("artist", "id")]),
            alias("target_id"),
        )
        .expr_as(
            Func::coalesce([
                column("redirect", "target_uuid"),
                column("artist", "source_uuid"),
            ]),
            alias("target_uuid"),
        )
        .from_as(catalog("artists"), alias("artist"))
        .join_as(
            JoinType::LeftJoin,
            catalog("artist_redirects"),
            alias("redirect"),
            column("redirect", "source_uuid").eq(column("artist", "source_uuid")),
        )
        .join_as(
            JoinType::LeftJoin,
            catalog("artists"),
            alias("target"),
            column("target", "source_uuid")
                .eq(column("redirect", "target_uuid"))
                .and(column("target", "portable").eq(1)),
        )
        .and_where(column("artist", "portable").eq(1))
        .to_owned()
}

fn current_album_titles() -> SelectStatement {
    // Select evidence from the cached summary or the live dirty projection.
    let mut q = Query::select();
    q.columns(["id", "legacy_id", "portable_title"].map(|name| (alias("album"), alias(name))))
        .from_as(catalog("albums"), alias("album"))
        .join_as(
            JoinType::LeftJoin,
            catalog("album_summaries"),
            alias("cached"),
            column("cached", "album_id").eq(column("album", "id")),
        )
        .join_as(
            JoinType::LeftJoin,
            catalog("dirty_summaries"),
            alias("dirty"),
            column("dirty", "album_id").eq(column("album", "id")),
        )
        .join_as(
            JoinType::LeftJoin,
            catalog("live_summaries"),
            alias("live"),
            column("live", "album_id").eq(column("dirty", "album_id")),
        );

    // Each effective field can mask the corresponding evidence value independently.
    for field in [
        "title",
        "version",
        "catalog_number",
        "release_date",
        "release_type",
    ] {
        let overlay = format!("effective_{field}");
        q.join_as(
            JoinType::LeftJoin,
            catalog("album_effective_fields"),
            alias(&overlay),
            column(&overlay, "album_id")
                .eq(column("album", "id"))
                .and(column(&overlay, "field").eq(field)),
        );
        let value: Expr = if field == "title" {
            Func::coalesce([
                column(&overlay, "value"),
                column("album", "portable_title"),
                column("live", field),
                column("cached", field),
                column("album", "directory_path"),
            ])
            .into()
        } else {
            Expr::case(
                column(&overlay, "album_id").is_not_null(),
                column(&overlay, "value"),
            )
            .case(
                column("live", "album_id").is_not_null(),
                column("live", field),
            )
            .finally(column("cached", field))
            .into()
        };
        q.expr_as(value, alias(field));
    }

    // A dirty summary without live evidence has no current tracks.
    let cached_count: Expr = Expr::case(
        column("dirty", "album_id").is_null(),
        column("cached", "track_count"),
    )
    .into();
    q.expr_as(
        Func::coalesce([column("live", "track_count"), cached_count, Expr::val(0)]),
        alias("track_count"),
    );
    q
}

fn browse_albums() -> SelectStatement {
    // Source identities stay addressable after their tracks resolve to local editions.
    let assigned = Query::select()
        .expr(Expr::val(1))
        .from_as(catalog("track_album_sources"), alias("source"))
        .join_as(
            JoinType::InnerJoin,
            catalog("audio"),
            alias("track"),
            column("track", "id").eq(column("source", "track_uuid")),
        )
        .and_where(column("source", "album_uuid").eq(column("album", "id")))
        .to_owned();
    let saved_id = Expr::case(
        column("album", "portable_title").is_not_null(),
        column("album", "legacy_id"),
    );

    // Show an edition once it has tracks, and keep empty albums from the old library.
    Query::select()
        .expr_as(
            Func::coalesce([saved_id.into(), Expr::val(0).sub(column("handle", "id"))]),
            alias("id"),
        )
        .expr_as(column("album", "title"), alias("name"))
        .expr_as(column("album", "id"), alias("source_uuid"))
        .from_as(catalog("current_album_titles"), alias("album"))
        .join_as(
            JoinType::InnerJoin,
            catalog("browse_handles"),
            alias("handle"),
            column("handle", "kind")
                .eq("album")
                .and(column("handle", "source_uuid").eq(column("album", "id"))),
        )
        .and_where(
            column("album", "track_count")
                .gt(0)
                .or(column("album", "portable_title")
                    .is_not_null()
                    .and(Expr::not_exists(assigned))),
        )
        .to_owned()
}

fn browse_artists() -> SelectStatement {
    // If a known identity uses the same spelling, keep the legacy artist only
    // while one of its tracks lacks a confirmed credit.
    let known = exists_in(
        catalog("artists"),
        Expr::col(alias("musicbrainz_id"))
            .is_not_null()
            .and(Expr::col(alias("name")).eq(nocase(column("artist", "name")))),
    );
    let redirects = exists_in(
        catalog("artist_redirects"),
        Expr::col(alias("source_uuid")).eq(column("artist", "source_uuid")),
    );
    let confirmed = exists_in(
        catalog("credits"),
        Expr::col(alias("file_id"))
            .eq(column("audio", "file_id"))
            .and(Expr::col(alias("scope")).eq(2))
            .and(Expr::col(alias("role")).eq("Primary Artist"))
            .and(Expr::col(alias("artist_id")).is_not_null())
            .and(Expr::col(alias("credited_as")).eq(nocase(column("artist", "name")))),
    );
    let unconfirmed = Query::select()
        .expr(Expr::val(1))
        .from_as(catalog("audio_artists"), alias("link"))
        .join_as(
            JoinType::LeftJoin,
            catalog("audio"),
            alias("audio"),
            column("audio", "id").eq(column("link", "track_uuid")),
        )
        .and_where(column("link", "artist_uuid").eq(column("artist", "source_uuid")))
        .and_where(Expr::not_exists(confirmed))
        .to_owned();

    // Preserve portable integer IDs while identified artists keep their browse handles.
    Query::select()
        .expr_as(
            Expr::case(column("artist", "portable").eq(1), column("artist", "id"))
                .finally(Expr::val(0).sub(column("handle", "id"))),
            alias("id"),
        )
        .column((alias("artist"), alias("name")))
        .expr_as(column("artist", "source_uuid"), alias("source_uuid"))
        .from_as(catalog("artists"), alias("artist"))
        .join_as(
            JoinType::InnerJoin,
            catalog("browse_handles"),
            alias("handle"),
            column("handle", "kind")
                .eq("artist")
                .and(column("handle", "source_uuid").eq(column("artist", "source_uuid"))),
        )
        .and_where(Expr::not_exists(redirects))
        .and_where(
            column("artist", "musicbrainz_id")
                .is_not_null()
                .or(column("artist", "portable")
                    .eq(1)
                    .and(Expr::not_exists(known).or(Expr::exists(unconfirmed)))),
        )
        .to_owned()
}
