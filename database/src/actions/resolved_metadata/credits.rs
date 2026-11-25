//! Storage for ordered, source-attributed credits and optional artist links.

use crate::entities::music_metadata::audio as orm_audio;
use crate::entities::music_metadata::credits as orm_credits;
use sea_orm::ExprTrait;
use sea_orm::sea_query::{Expr, Order, Query};
use std::collections::BTreeSet;

use anyhow::{Context, Result};
use metadata::{
    model::{Credit, Credits, FieldSource, MetadataBase, MetadataSourceKind},
    resolve::LocalMetadata,
};
use sea_orm::{ColumnTrait, ConnectionTrait, DatabaseTransaction, QueryResult, Value};
use uuid::Uuid;

use super::{
    CreditScope,
    details::{Contributors, Selection},
};

const SOURCE_KINDS: &[(&str, MetadataSourceKind)] = &[
    ("audio_tag", MetadataSourceKind::AudioTag),
    ("audio_stream", MetadataSourceKind::AudioStream),
    ("filename", MetadataSourceKind::Filename),
    ("directory", MetadataSourceKind::Directory),
    ("sidecar", MetadataSourceKind::Sidecar),
    ("embedded_artwork", MetadataSourceKind::EmbeddedArtwork),
    ("aggregate", MetadataSourceKind::Aggregate),
    ("external", MetadataSourceKind::External),
];

pub(super) async fn store(
    db: &DatabaseTransaction,
    file_id: i32,
    metadata: &LocalMetadata,
) -> Result<()> {
    let album_artist =
        super::artist_identity::primary(db, &metadata.album.base, "musicbrainz:album-artist")
            .await?;
    let audio_artist =
        super::artist_identity::primary(db, &metadata.audio.base, "musicbrainz:artist").await?;
    let mut rows = Vec::new();
    for (scope, base, identified) in [
        (CreditScope::Album, &metadata.album.base, album_artist),
        (CreditScope::Disc, &metadata.disc.base, None),
        (CreditScope::Audio, &metadata.audio.base, audio_artist),
    ] {
        for (position, credit) in base.credits.iter().flatten().enumerate() {
            let artist = credit.artist_id.map(|id| id.to_string()).or_else(|| {
                identified
                    .clone()
                    .filter(|_| credit.role == "Primary Artist")
            });
            rows.push(bind(
                file_id,
                scope,
                position.try_into()?,
                credit,
                artist,
                "",
            )?);
        }
    }

    // The caller's scan transaction replaces the evidence and its links together.
    db.execute(
        Query::delete()
            .from_table(orm_credits::Entity)
            .and_where(Expr::col(orm_credits::Column::FileId).eq(Expr::val(file_id)))
            .and_where(Expr::col(orm_credits::Column::SourceKind).eq(Expr::val("audio_tag")))
            .and_where(Expr::col(orm_credits::Column::SourceProvider).eq(Expr::val(""))),
    )
    .await?;
    insert(db, &rows).await
}

async fn insert(db: &DatabaseTransaction, rows: &[[Value; 14]]) -> Result<()> {
    use orm_credits::Column as C;
    for rows in rows.chunks(64) {
        let mut insert = Query::insert();
        insert.into_table(orm_credits::Entity).columns([
            C::FileId,
            C::Scope,
            C::Position,
            C::CreditedAs,
            C::Role,
            C::ArtistId,
            C::Instrument,
            C::SourceKind,
            C::SourceProvider,
            C::SourceKey,
            C::SourcePath,
            C::SourcePattern,
            C::SourceEntityId,
            C::SourceConfidence,
        ]);
        for row in rows {
            insert.values(row.iter().cloned().map(Expr::val))?;
        }
        db.execute(&insert).await?;
    }
    Ok(())
}

fn bind(
    file_id: i32,
    scope: CreditScope,
    position: i64,
    credit: &Credit,
    artist: Option<String>,
    provider: &str,
) -> Result<[Value; 14]> {
    let source = &credit.source;
    let kind = source_name(source.kind)?;
    Ok([
        file_id.into(),
        (scope as i32).into(),
        position.into(),
        credit.credited_as.clone().into(),
        credit.role.clone().into(),
        artist.into(),
        credit.instrument.clone().into(),
        kind.into(),
        provider.to_owned().into(),
        // A credit retains its own origin after album-level deduplication.
        source.key.clone().into(),
        source.path.clone().into(),
        source.pattern.clone().into(),
        source.entity_id.map(|id| id.to_string()).into(),
        source.confidence.into(),
    ])
}

pub(super) fn source_name(source: MetadataSourceKind) -> Result<&'static str> {
    Ok(SOURCE_KINDS
        .iter()
        .find(|(_, kind)| *kind == source)
        .context("unsupported credit source kind")?
        .0)
}

/// Refresh a single provider source in the caller's write transaction.
pub(super) async fn replace_source(
    db: &DatabaseTransaction,
    track_id: Uuid,
    scope: CreditScope,
    source: MetadataSourceKind,
    provider: &str,
    credits: &[Credit],
) -> Result<bool> {
    anyhow::ensure!(
        source != MetadataSourceKind::AudioTag,
        "audio tag credits are owned by the scanner"
    );
    let provider = provider.trim();
    anyhow::ensure!(!provider.is_empty(), "credit provider is required");
    let Some(track) = db
        .query_one(
            Query::select()
                .column(orm_audio::Column::FileId)
                .from(orm_audio::Entity)
                .and_where(Expr::col(orm_audio::Column::Id).eq(Expr::val(track_id.to_string()))),
        )
        .await?
    else {
        return Ok(false);
    };

    // Validate and prepare rows before replacing the provider's previous contribution.
    let file_id: i32 = track.try_get("", "file_id")?;
    let rows = credits
        .iter()
        .enumerate()
        .map(|(position, credit)| {
            anyhow::ensure!(
                credit.source.kind == source
                    && credit
                        .source_provider
                        .as_deref()
                        .is_none_or(|existing| existing == provider),
                "credit source does not match its provider"
            );
            bind(
                file_id,
                scope,
                position.try_into()?,
                credit,
                credit.artist_id.map(|id| id.to_string()),
                provider,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    db.execute(
        Query::delete()
            .from_table(orm_credits::Entity)
            .and_where(Expr::col(orm_credits::Column::FileId).eq(Expr::val(file_id)))
            .and_where(Expr::col(orm_credits::Column::Scope).eq(Expr::val(scope as i32)))
            .and_where(
                Expr::col(orm_credits::Column::SourceKind).eq(Expr::val(source_name(source)?)),
            )
            .and_where(
                Expr::col(orm_credits::Column::SourceProvider).eq(Expr::val(provider.to_owned())),
            ),
    )
    .await?;
    insert(db, &rows).await?;
    Ok(true)
}

pub(super) fn read(row: &QueryResult) -> Result<Credit> {
    let kind: String = row.try_get("", "source_kind")?;
    let (_, kind) = SOURCE_KINDS
        .iter()
        .find(|(name, _)| *name == kind)
        .context("unsupported stored credit source kind")?;
    let source = FieldSource {
        kind: *kind,
        key: row.try_get("", "source_key")?,
        path: row.try_get("", "source_path")?,
        pattern: row.try_get("", "source_pattern")?,
        entity_id: optional_uuid(row, "source_entity_id")?,
        confidence: row.try_get("", "source_confidence")?,
    };
    let provider: String = row.try_get("", "source_provider")?;
    Ok(Credit {
        credited_as: row.try_get("", "credited_as")?,
        role: row.try_get("", "role")?,
        artist_id: optional_uuid(row, "artist_id")?,
        instrument: row.try_get("", "instrument")?,
        source,
        source_provider: (!provider.is_empty()).then_some(provider),
    })
}

fn optional_uuid(row: &QueryResult, column: &str) -> Result<Option<Uuid>> {
    Ok(row
        .try_get::<Option<String>>("", column)?
        .as_deref()
        .map(str::parse)
        .transpose()?)
}

pub(super) async fn load(
    db: &impl ConnectionTrait,
    selection: Selection,
    first_file: Option<i32>,
    base: &mut MetadataBase,
    contributors: &mut Contributors,
) -> Result<()> {
    use orm_credits::Column as C;
    let mut query = selection.rows(orm_credits::Entity, C::FileId, C::Scope);
    let priority = Expr::case(C::SourceKind.eq("audio_tag"), 0).finally(1);
    query
        .columns(
            [
                C::CreditedAs,
                C::Role,
                C::Instrument,
                C::SourceKind,
                C::SourceProvider,
                C::SourceKey,
                C::SourcePath,
                C::SourcePattern,
                C::SourceEntityId,
                C::SourceConfidence,
            ]
            .map(|c| c.as_column_ref()),
        )
        .column(C::ArtistId.as_column_ref())
        .order_by_expr(priority.into(), Order::Asc)
        .order_by_columns(
            [C::SourceKind, C::SourceProvider, C::Position]
                .map(|c| (c.as_column_ref(), Order::Asc)),
        );
    let rows = db.query_all(&query).await?;
    let mut credits = Credits::new();

    // Deduplicate album credits without conflating two confirmed artists with the same name.
    let mut seen = BTreeSet::new();
    for row in rows {
        let file_id: i32 = row.try_get("", "file_id")?;
        if first_file.is_some_and(|first| first != file_id) {
            continue;
        }
        let credit = read(&row)?;
        let key = (
            credit.credited_as.clone(),
            credit.role.clone(),
            credit.instrument.clone(),
            credit.artist_id,
            source_name(credit.source.kind)?,
            credit.source_provider.clone(),
        );
        if matches!(selection, Selection::Audio(_)) || seen.insert(key) {
            contributors
                .entry(file_id)
                .or_default()
                .insert(format!("credits.{}", credit.role));
            credits.push(credit);
        }
    }
    base.credits = (!credits.is_empty()).then_some(credits);
    Ok(())
}
