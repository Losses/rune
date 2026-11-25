//! Composable selections from the catalog's stored evidence.

use sea_orm::{
    ColumnTrait, ExprTrait, JoinType,
    sea_query::{
        Alias, Asterisk, BinOper, Expr, Func, Query, SelectStatement, TableRef, UnionType,
    },
};

use crate::entities::{
    media_files,
    music_metadata::{audio, recording_sources, recordings, release_tracks, track_assets},
};

use super::{column, scalar};

pub(crate) fn subquery(query: SelectStatement, name: &str) -> TableRef {
    TableRef::SubQuery(Box::new(query), Alias::new(name).into())
}

pub(crate) fn field(table: &str, name: &str) -> Expr {
    Expr::col((Alias::new(table), Alias::new(name)))
}

pub(super) fn matches_content() -> Expr {
    use audio::Column as A;
    use media_files::Column as F;
    column(A::RecordingContentHash)
        .binary(BinOper::Is, column(F::ContentHash))
        .and(column(A::RecordingContentSize).binary(BinOper::Is, column(F::ContentSize)))
}

pub(crate) fn verified_audio() -> TableRef {
    use audio::Column as A;
    use media_files::Column as F;
    let rows = Query::select()
        .column((audio::Entity, Asterisk))
        .from(audio::Entity)
        .inner_join(media_files::Entity, column(F::Id).eq(column(A::FileId)))
        .and_where(matches_content())
        .to_owned();
    subquery(rows, "music_metadata_audio")
}

pub(super) const EVIDENCE: &str = "recording_evidence";

pub(super) fn recording_evidence() -> TableRef {
    use audio::Column as A;
    use recording_sources::Column as S;
    // Providers have no indexed file. The UNION gives both sources the same fields.
    let providers = Query::select()
        .exprs([Expr::val(None::<i32>), Expr::val(None::<String>)])
        .columns([
            S::Provider,
            S::ResourceId,
            S::RecordingId,
            S::RecordingMbid,
            S::RecordingIsrc,
        ])
        .from(recording_sources::Entity)
        .to_owned();
    let rows = Query::select()
        .column(A::FileId)
        .expr_as(column(A::Id), "audio_id")
        .expr_as(Expr::val(None::<String>), "provider")
        .expr_as(Expr::val(None::<String>), "resource_id")
        .column(A::RecordingId)
        .expr_as(column(A::RecordingMbid), "mbid")
        .expr_as(column(A::RecordingIsrc), "isrc")
        .from(verified_audio())
        .union(UnionType::All, providers)
        .to_owned();
    subquery(rows, EVIDENCE)
}

pub(super) const TRACK_RECORDINGS: &str = "track_recordings";

pub(super) fn track_recordings() -> TableRef {
    use audio::Column as A;
    use recordings::Column as R;
    use release_tracks::Column as T;
    use track_assets::Column as V;

    // Only a complete file can identify a recording and all evidence must agree.
    let verified = Query::select()
        .expr(Func::min(column(A::RecordingId)))
        .from(track_assets::Entity)
        .inner_join(verified_audio(), column(A::FileId).eq(column(V::FileId)))
        .inner_join(recordings::Entity, column(R::Id).eq(column(A::RecordingId)))
        .and_where(column(V::TrackId).eq(column(T::Id)))
        .and_where(V::StartMs.eq(0))
        .and_where(V::EndMs.is_null())
        .and_where(R::MusicbrainzId.is_not_null().or(R::Isrc.is_not_null()))
        .and_having(Expr::from(Func::count_distinct(column(A::RecordingId))).eq(1))
        .to_owned();
    let rows = Query::select()
        .expr_as(column(T::Id), "track_id")
        .expr_as(
            Func::coalesce([scalar(verified), column(T::RecordingId)]),
            "recording_id",
        )
        .from(release_tracks::Entity)
        .to_owned();
    subquery(rows, TRACK_RECORDINGS)
}

pub(super) const RECORDING_ALIASES: &str = "recording_aliases";

pub(super) fn recording_aliases() -> TableRef {
    use recording_sources::Column as S;
    use recordings::Column as R;
    use release_tracks::Column as T;
    let current = Alias::new("current");
    let current_id = Expr::col((current.clone(), R::Id));
    let current_isrc = Expr::col((current.clone(), R::Isrc));

    // A previous ISRC-only identity can be followed only when there is a single match.
    let conflicting = Query::select()
        .column(R::Id)
        .from_as(recordings::Entity, "conflict")
        .and_where(field("conflict", "isrc").eq(current_isrc.clone()))
        .and_where(field("conflict", "id").ne(current_id.clone()))
        .to_owned();
    let logical = Query::select()
        .column(T::RecordingId.as_column_ref())
        .expr(field(TRACK_RECORDINGS, "recording_id"))
        .from(release_tracks::Entity)
        .inner_join(
            track_recordings(),
            field(TRACK_RECORDINGS, "track_id").eq(column(T::Id)),
        )
        .and_where(field(TRACK_RECORDINGS, "recording_id").ne(column(T::RecordingId)))
        .to_owned();

    // Keep provider credits attached to their fallback while the provider resolves to a match.
    let providers = Query::select()
        .columns([S::FallbackId, S::RecordingId])
        .from(recording_sources::Entity)
        .and_where(column(S::FallbackId).ne(column(S::RecordingId)))
        .to_owned();
    let rows = Query::select()
        .expr_as(column(R::Id), "source_id")
        .expr_as(current_id.clone(), "target_id")
        .from(recordings::Entity)
        .join_as(
            JoinType::InnerJoin,
            recordings::Entity,
            current,
            current_isrc.eq(column(R::LegacyIsrc)),
        )
        .and_where(R::Isrc.is_null())
        .and_where(current_id.ne(column(R::Id)))
        .and_where(Expr::not_exists(conflicting))
        .union(UnionType::All, logical)
        .union(UnionType::All, providers)
        .to_owned();
    subquery(rows, RECORDING_ALIASES)
}
