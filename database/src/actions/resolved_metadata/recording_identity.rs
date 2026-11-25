//! Reconcile normalized identifier evidence without letting a partial tag seed a match.

use crate::entities::music_metadata::audio as orm_audio;
use crate::entities::music_metadata::recording_dirty as orm_recording_dirty;
use crate::entities::music_metadata::recordings as orm_recordings;
use sea_orm::ExprTrait;
use sea_orm::sea_query::{
    Alias, Asterisk, CommonTableExpression, Expr, Func, IntoTableRef, OnConflict, Query,
    SelectStatement, UnionType, WithClause,
};
use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use metadata::model::RecordingIdentifiers;
use sea_orm::{ColumnTrait, ConnectionTrait, QueryResult};
use uuid::Uuid;

use crate::entities::music_metadata::recording_sources;

use super::{
    column,
    queries::{EVIDENCE, field, recording_evidence},
    uuid,
};

pub(super) const NAMESPACE: Uuid = Uuid::from_u128(0x59e7e858_d294_50cc_9468_d2b63f436799);

pub(super) fn audio_fallback(audio: Uuid) -> Uuid {
    Uuid::new_v5(&audio, b"recording")
}

pub(super) fn remote_fallback(provider: &str, resource: &str) -> Uuid {
    let provider = Uuid::new_v5(&NAMESPACE, provider.as_bytes());
    Uuid::new_v5(&provider, resource.as_bytes())
}

pub(super) fn isrc(value: &str) -> Option<String> {
    let value = value.trim().replace('-', "").to_ascii_uppercase();
    let bytes = value.as_bytes();
    (bytes.len() == 12
        && bytes[..2].iter().all(u8::is_ascii_uppercase)
        && bytes[2..5].iter().all(u8::is_ascii_alphanumeric)
        && bytes[5..].iter().all(u8::is_ascii_digit))
    .then_some(value)
}

pub(super) async fn ensure_exists(db: &impl ConnectionTrait, id: Uuid) -> Result<()> {
    db.execute(
        Query::insert()
            .into_table(orm_recordings::Entity)
            .columns([orm_recordings::Column::Id])
            .values_panic([Expr::val(id.to_string())])
            .on_conflict(
                OnConflict::column(orm_recordings::Column::Id)
                    .do_nothing()
                    .to_owned(),
            ),
    )
    .await?;
    Ok(())
}

pub(super) async fn prune_unreferenced(
    db: &impl ConnectionTrait,
    ids: BTreeSet<Uuid>,
) -> Result<()> {
    use crate::entities::music_metadata::{recording_credits, release_tracks};
    use orm_recordings::Column as R;
    use recording_sources::Column as S;
    if ids.is_empty() {
        return Ok(());
    }

    // Retire replaced fallback rows only after every asset has been reassigned.
    let mut remove = Query::delete();
    remove
        .from_table(orm_recordings::Entity)
        .and_where(R::Id.is_in(ids.into_iter().map(|id| id.to_string())))
        .and_where(R::MusicbrainzId.is_null())
        .and_where(R::Isrc.is_null())
        .and_where(R::LegacyIsrc.is_null());
    for (table, field) in [
        (
            orm_audio::Entity.into_table_ref(),
            orm_audio::Column::RecordingId.as_column_ref(),
        ),
        (
            release_tracks::Entity.into_table_ref(),
            release_tracks::Column::RecordingId.as_column_ref(),
        ),
        (
            recording_credits::Entity.into_table_ref(),
            recording_credits::Column::RecordingId.as_column_ref(),
        ),
        (
            recording_sources::Entity.into_table_ref(),
            S::RecordingId.as_column_ref(),
        ),
        (
            recording_sources::Entity.into_table_ref(),
            S::FallbackId.as_column_ref(),
        ),
    ] {
        let reference = Query::select()
            .expr(Expr::val(1))
            .from(table)
            .and_where(Expr::col(field).eq(column(R::Id)))
            .to_owned();
        remove.and_where(Expr::not_exists(reference));
    }
    db.execute(&remove).await?;
    Ok(())
}

pub(super) async fn prepare_audio(
    db: &impl ConnectionTrait,
    audio: Uuid,
    ids: &RecordingIdentifiers,
    content: (Option<&str>, Option<i64>),
) -> Result<Uuid> {
    // Read both the previous identifiers and the version that asserted them.
    use orm_audio::Column as A;
    let before = db
        .query_one(
            Query::select()
                .columns([
                    A::RecordingId,
                    A::RecordingMbid,
                    A::RecordingIsrc,
                    A::RecordingContentHash,
                    A::RecordingContentSize,
                ])
                .from(orm_audio::Entity)
                .and_where(A::Id.eq(audio.to_string())),
        )
        .await?;
    let version_changed = if let Some(row) = &before {
        row.try_get::<Option<String>>("", "recording_content_hash")?
            .as_deref()
            != content.0
            || row.try_get::<Option<i64>>("", "recording_content_size")? != content.1
    } else {
        false
    };

    prepare_source(
        db,
        before.as_ref(),
        audio_fallback(audio),
        ids,
        version_changed,
    )
    .await
}

pub(super) async fn prepare_source(
    db: &impl ConnectionTrait,
    before: Option<&QueryResult>,
    fallback: Uuid,
    ids: &RecordingIdentifiers,
    version_changed: bool,
) -> Result<Uuid> {
    let mbid = ids.musicbrainz_id.map(|id| id.to_string());
    let isrc = ids.isrc.as_deref();
    let mut recording = fallback;
    if let Some(before) = before {
        let previous_id = uuid(before, "recording_id")?;
        let previous_mbid: Option<String> = before.try_get("", "recording_mbid")?;
        let previous_isrc: Option<String> = before.try_get("", "recording_isrc")?;
        if previous_mbid == mbid && previous_isrc.as_deref() == isrc {
            if !version_changed {
                return Ok(previous_id);
            }
            recording = previous_id;
        }

        // Revoke old evidence as well as scheduling the replacement.
        queue(
            db,
            previous_id,
            previous_mbid.as_deref(),
            previous_isrc.as_deref(),
        )
        .await?;
    }
    if recording == fallback {
        ensure_exists(db, fallback).await?;
    }
    queue(db, recording, mbid.as_deref(), isrc).await?;
    Ok(recording)
}

async fn queue(
    db: &impl ConnectionTrait,
    id: Uuid,
    mbid: Option<&str>,
    isrc: Option<&str>,
) -> Result<()> {
    let id = id.to_string();
    let mut rows = Query::insert();
    use orm_recording_dirty::Column as Dirty;
    rows.into_table(orm_recording_dirty::Entity)
        .columns([Dirty::Kind, Dirty::Value]);
    for (kind, value) in [
        ("recording", Some(id.as_str())),
        ("mbid", mbid),
        ("isrc", isrc),
    ] {
        if let Some(value) = value {
            rows.values_panic([Expr::val(kind), Expr::val(value)]);
        }
    }

    // Repeated observations of the same identifier need a single reconciliation.
    db.execute(
        rows.on_conflict(
            OnConflict::columns([Dirty::Kind, Dirty::Value])
                .do_nothing()
                .to_owned(),
        ),
    )
    .await?;
    Ok(())
}

pub(super) async fn invalidate_stale_file(db: &impl ConnectionTrait, track: &str) -> Result<()> {
    use crate::entities::media_files;
    use media_files::Column as F;
    use orm_audio::Column as A;

    // The source-row projection has already updated the file's content version.
    let previous = db
        .query_one(
            Query::select()
                .columns(
                    [A::RecordingId, A::RecordingMbid, A::RecordingIsrc].map(|c| c.as_column_ref()),
                )
                .from(orm_audio::Entity)
                .inner_join(media_files::Entity, column(F::Id).eq(column(A::FileId)))
                .and_where(A::Id.eq(track))
                .and_where(super::queries::matches_content().not()),
        )
        .await?;
    if let Some(previous) = previous {
        let mbid: Option<String> = previous.try_get("", "recording_mbid")?;
        let isrc: Option<String> = previous.try_get("", "recording_isrc")?;
        queue(
            db,
            uuid(&previous, "recording_id")?,
            mbid.as_deref(),
            isrc.as_deref(),
        )
        .await?;
    }
    Ok(())
}

#[derive(Debug)]
enum Owner {
    Audio { file: i32, id: Uuid },
    Remote { provider: String, resource: String },
}

struct Evidence {
    owner: Owner,
    current: Uuid,
    mbid: Option<String>,
    isrc: Option<String>,
}

impl Evidence {
    fn read(row: &QueryResult) -> Result<Self> {
        let owner = if let Some(file) = row.try_get::<Option<i32>>("", "file_id")? {
            Owner::Audio {
                file,
                id: uuid(row, "audio_id")?,
            }
        } else {
            Owner::Remote {
                provider: row.try_get("", "provider")?,
                resource: row.try_get("", "resource_id")?,
            }
        };
        Ok(Self {
            owner,
            current: uuid(row, "recording_id")?,
            mbid: row.try_get("", "mbid")?,
            isrc: row.try_get("", "isrc")?,
        })
    }

    fn fallback(&self) -> Uuid {
        match &self.owner {
            Owner::Audio { id, .. } => audio_fallback(*id),
            Owner::Remote { provider, resource } => remote_fallback(provider, resource),
        }
    }
}

type Claims<'a> = BTreeMap<&'a str, BTreeSet<&'a str>>;

/// Rebuild only identifier components touched by source changes in the current transaction.
pub(in crate::actions) async fn reconcile(db: &impl ConnectionTrait) -> Result<()> {
    if db
        .query_one(
            Query::select()
                .expr(Expr::val(1))
                .from(orm_recording_dirty::Entity)
                .limit(1),
        )
        .await?
        .is_none()
    {
        return Ok(());
    }
    // The bipartite identifier graph expands equally from the old and new identifiers.
    let rows = db.query_all(&related_evidence()).await?;
    let evidence = rows
        .iter()
        .map(Evidence::read)
        .collect::<Result<Vec<_>>>()?;
    let (mut by_mbid, mut by_isrc) = (Claims::new(), Claims::new());
    for entry in &evidence {
        if let (Some(mbid), Some(isrc)) = (entry.mbid.as_deref(), entry.isrc.as_deref()) {
            by_mbid.entry(mbid).or_default().insert(isrc);
            by_isrc.entry(isrc).or_default().insert(mbid);
        }
    }

    // Clear derived identifiers while retaining the identity and its credited history.
    use orm_recordings::Column as R;
    let affected = |property| {
        Query::select()
            .column(orm_recording_dirty::Column::Value)
            .from(orm_recording_dirty::Entity)
            .and_where(orm_recording_dirty::Column::Kind.eq(property))
            .to_owned()
    };
    db.execute(
        Query::update()
            .table(orm_recordings::Entity)
            .value(R::MusicbrainzId, Expr::val(None::<String>))
            .value(R::Isrc, Expr::val(None::<String>))
            .and_where(
                column(R::Id)
                    .in_subquery(affected("recording"))
                    .or(column(R::MusicbrainzId).in_subquery(affected("mbid")))
                    .or(column(R::Isrc).in_subquery(affected("isrc"))),
            ),
    )
    .await?;
    let mut known = BTreeSet::new();
    for entry in &evidence {
        let (id, mbid, isrc) = classify(entry, &by_mbid, &by_isrc);
        if known.insert(id) {
            update_projection(db, id, mbid, isrc).await?;
        }
        if id != entry.current {
            attach(db, &entry.owner, id).await?;
        }
    }
    db.execute(Query::delete().from_table(orm_recording_dirty::Entity))
        .await?;
    Ok(())
}

fn related_evidence() -> SelectStatement {
    use orm_recording_dirty::Column as D;
    let keys = Alias::new("keys");
    let kind = Expr::col((keys.clone(), D::Kind));
    let value = Expr::col((keys.clone(), D::Value));

    // Traverse both identifier types until the touched components stop growing.
    let edges = |from, to, from_kind, to_kind| {
        Query::select()
            .expr(Expr::val(to_kind))
            .column((Alias::new(EVIDENCE), Alias::new(to)))
            .from(keys.clone())
            .inner_join(
                recording_evidence(),
                field(EVIDENCE, from).eq(value.clone()),
            )
            .and_where(kind.clone().eq(from_kind))
            .and_where(field(EVIDENCE, to).is_not_null())
            .to_owned()
    };
    let seed = Query::select()
        .columns([D::Kind, D::Value])
        .from(orm_recording_dirty::Entity)
        .and_where(D::Kind.ne("recording"))
        .union(UnionType::Distinct, edges("mbid", "isrc", "mbid", "isrc"))
        .union(UnionType::Distinct, edges("isrc", "mbid", "isrc", "mbid"))
        .to_owned();
    let cte = CommonTableExpression::new()
        .table_name(keys.clone())
        .columns([D::Kind, D::Value])
        .query(seed)
        .to_owned();

    // Deduplicate rows that can be reached through both identifier paths.
    let matches = |identifier, identifier_kind| {
        Query::select()
            .column((Alias::new(EVIDENCE), Asterisk))
            .from(keys.clone())
            .inner_join(
                recording_evidence(),
                field(EVIDENCE, identifier).eq(value.clone()),
            )
            .and_where(kind.clone().eq(identifier_kind))
            .to_owned()
    };
    matches("mbid", "mbid")
        .union(UnionType::Distinct, matches("isrc", "isrc"))
        .with_cte(WithClause::new().recursive(true).cte(cte).to_owned())
        .to_owned()
}

fn classify<'a>(
    entry: &'a Evidence,
    by_mbid: &Claims<'a>,
    by_isrc: &Claims<'a>,
) -> (Uuid, Option<&'a str>, Option<&'a str>) {
    let isrc = entry.isrc.as_deref();
    let mbid = match (
        entry.mbid.as_deref(),
        isrc.and_then(|isrc| by_isrc.get(isrc)),
    ) {
        (Some(mbid), _) => Some(mbid),
        (None, Some(candidates)) if candidates.len() == 1 => candidates.first().copied(),
        (None, Some(_)) => return (entry.fallback(), None, None),
        (None, None) => None,
    };

    // Exact contradictory pairs stay distinct. Unqualified evidence cannot decide between them.
    if let Some(mbid) = mbid {
        let candidates = by_mbid.get(mbid);
        if candidates.is_some_and(|isrcs| isrcs.len() > 1) {
            let Some(isrc) = isrc else {
                return (entry.fallback(), None, None);
            };
            return (
                Uuid::new_v5(&NAMESPACE, format!("mb:{mbid}:{isrc}").as_bytes()),
                Some(mbid),
                Some(isrc),
            );
        }
        let isrc = candidates.and_then(BTreeSet::first).copied().or(isrc);
        return (
            Uuid::new_v5(&NAMESPACE, format!("mb:{mbid}:").as_bytes()),
            Some(mbid),
            isrc,
        );
    }
    if let Some(isrc) = isrc {
        return (
            Uuid::new_v5(&NAMESPACE, format!("isrc:{isrc}").as_bytes()),
            None,
            Some(isrc),
        );
    }
    (entry.fallback(), None, None)
}

async fn update_projection(
    db: &impl ConnectionTrait,
    id: Uuid,
    mbid: Option<&str>,
    isrc: Option<&str>,
) -> Result<()> {
    use orm_recordings::Column as R;
    let legacy_isrc = Func::coalesce([
        column(R::LegacyIsrc),
        Expr::col((Alias::new("excluded"), R::LegacyIsrc)),
    ]);
    db.execute(
        Query::insert()
            .into_table(orm_recordings::Entity)
            .columns([R::Id, R::MusicbrainzId, R::Isrc, R::LegacyIsrc])
            .values([
                Expr::val(id.to_string()),
                Expr::val(mbid),
                Expr::val(isrc),
                Expr::val(isrc.filter(|_| mbid.is_none())),
            ])?
            .on_conflict(
                OnConflict::column(R::Id)
                    .update_columns([R::MusicbrainzId, R::Isrc])
                    .value(R::LegacyIsrc, legacy_isrc)
                    .to_owned(),
            ),
    )
    .await?;
    Ok(())
}

async fn attach(db: &impl ConnectionTrait, owner: &Owner, id: Uuid) -> Result<()> {
    let id = id.to_string();
    let query = match owner {
        Owner::Audio { file, .. } => Query::update()
            .table(orm_audio::Entity)
            .value(orm_audio::Column::RecordingId, Expr::val(id))
            .and_where(orm_audio::Column::FileId.eq(*file))
            .to_owned(),
        Owner::Remote { provider, resource } => Query::update()
            .table(recording_sources::Entity)
            .value(recording_sources::Column::RecordingId, Expr::val(id))
            .and_where(recording_sources::Column::Provider.eq(provider))
            .and_where(recording_sources::Column::ResourceId.eq(resource))
            .to_owned(),
    };
    db.execute(&query).await?;
    Ok(())
}
