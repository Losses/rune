//! Project portable source rows into the current browsing catalog.

use std::collections::BTreeMap;

use migration::catalog_identity::{ParentKind, parent};
use rust_decimal::prelude::ToPrimitive;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbErr, EntityTrait, ExprTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
    sea_query::{Expr, OnConflict, Query},
};
use uuid::Uuid;

use crate::entities::{
    media_files,
    music_metadata::{
        album_evidence, albums, audio, credits, disc_evidence, discs, native_audio, recordings,
        values,
    },
    portable_albums, source_tags, track_album_sources,
};

use super::{CreditScope, column, queries, recording_identity, tracks};

#[cfg(test)]
mod tests;

const CHUNK: usize = 192;

pub(crate) const TAG_LIMIT: usize = 128;
pub(crate) const TAG_BYTES: usize = 16384;
const LIST_BYTES: usize = sync::protocol::MAX_CANDIDATE_BYTES as usize;
// One bounded tag lookup plus scalar reads from the projection and its lifecycle hooks.
pub(crate) const READ_LIMIT: u64 = TAG_LIMIT as u64 + 1 + 8;
// A genre needs one byte and two separator bytes, except the first in each tag.
const GENRE_LIMIT: usize = (LIST_BYTES + 2 * TAG_LIMIT) / 3;
// Seven scalar writes, two list deletes, one invalidation and up to seven lifecycle writes.
pub(crate) const MUTATION_LIMIT: u64 =
    17 + (TAG_LIMIT.div_ceil(CHUNK) + GENRE_LIMIT.div_ceil(CHUNK)) as u64;

pub(super) async fn project(db: &impl ConnectionTrait, track: Uuid) -> Result<Option<i32>, DbErr> {
    use media_files::Column as F;
    let native = Query::select()
        .expr(Expr::val(1))
        .from(native_audio::Entity)
        .inner_join(
            audio::Entity,
            column(audio::Column::FileId).eq(column(native_audio::Column::FileId)),
        )
        .and_where(column(audio::Column::FileId).eq(column(F::Id)))
        .and_where(queries::matches_content())
        .to_owned();
    let Some(file) = media_files::Entity::find()
        .filter(F::SourceUuid.eq(track.to_string()))
        .filter(F::LocalAvailable.eq(false))
        .filter(Expr::not_exists(native))
        .one(db)
        .await?
    else {
        return Ok(None);
    };

    // Read bounded source lists in the same order used by synchronization.
    let tags = source_tags::Entity::find()
        .filter(source_tags::Column::FileId.eq(file.id))
        .order_by_asc(source_tags::Column::Position)
        .order_by_asc(source_tags::Column::Id)
        .limit((TAG_LIMIT + 1) as u64)
        .all(db)
        .await?;
    if tags.len() > TAG_LIMIT
        || tags.iter().map(|tag| tag.meta_value.len()).sum::<usize>() > LIST_BYTES
        || tags.iter().any(|tag| tag.meta_value.len() > TAG_BYTES)
    {
        return Err(DbErr::Custom("synced tags exceed projection limits".into()));
    }
    let mut fields = BTreeMap::<_, Vec<_>>::new();
    for tag in &tags {
        fields
            .entry(tag.meta_key.as_str())
            .or_default()
            .push(tag.meta_value.as_str());
    }

    // The authored membership is available independently of the album record.
    let assignment = track_album_sources::Entity::find()
        .filter(track_album_sources::Column::TrackUuid.eq(&file.source_uuid))
        .one(db)
        .await?;
    let source_album = if let Some(link) = &assignment {
        portable_albums::Entity::find()
            .filter(portable_albums::Column::SourceUuid.eq(&link.album_uuid))
            .one(db)
            .await?
    } else {
        None
    };
    let source = |key| {
        fields
            .get(key)?
            .last()
            .copied()
            .filter(|value| !value.trim().is_empty())
    };
    let title = source("track_title")
        .or_else(|| source("title"))
        .unwrap_or(&file.file_name);
    let tag_title = source_album
        .as_ref()
        .map(|row| row.name.as_str())
        .or_else(|| source("album"));
    let album_title = tag_title.unwrap_or_default();
    let edition = assignment.as_ref().map_or_else(
        || {
            Uuid::new_v5(
                &parent(ParentKind::Album, album_title),
                file.directory.as_bytes(),
            )
            .to_string()
        },
        |link| link.album_uuid.clone(),
    );

    // Prefer explicit positions and use the packed legacy number as a fallback.
    let number = |key| {
        source(key)?
            .split('/')
            .next()?
            .trim()
            .parse::<i32>()
            .ok()
            .filter(|n| *n >= 0)
    };
    let indexed = assignment
        .as_ref()
        .and_then(|link| link.track_number)
        .filter(|n| *n >= 0);
    let disc_number = number("disc_number")
        .or_else(|| indexed.map(|n| n / 1000))
        .filter(|n| *n > 0);
    let track_number = number("track_number").or_else(|| indexed.map(|n| n % 1000));
    let medium = disc_number.unwrap_or(1);
    let edition_uuid = Uuid::parse_str(&edition).map_err(|e| DbErr::Custom(e.to_string()))?;
    let disc = Uuid::new_v5(&edition_uuid, &medium.to_be_bytes()).to_string();
    let recording = recording_identity::audio_fallback(track).to_string();

    // Write the derived parents before replacing the file's catalog evidence.
    albums::Entity::insert(albums::ActiveModel {
        id: Set(edition.clone()),
        directory_path: Set(file.directory.clone()),
        managed_key: Set(assignment.as_ref().map(|_| format!("source:{edition}"))),
        ..Default::default()
    })
    .on_conflict_do_nothing()
    .exec_without_returning(db)
    .await?;
    discs::Entity::insert(discs::ActiveModel {
        id: Set(disc.clone()),
        album_id: Set(edition),
        disc_number: Set(medium),
    })
    .on_conflict_do_nothing()
    .exec_without_returning(db)
    .await?;
    recordings::Entity::insert(recordings::ActiveModel {
        id: Set(recording.clone()),
        ..Default::default()
    })
    .on_conflict_do_nothing()
    .exec_without_returning(db)
    .await?;

    use audio::Column as A;
    audio::Entity::insert(audio::ActiveModel {
        file_id: Set(file.id),
        id: Set(file.source_uuid),
        disc_id: Set(disc),
        recording_id: Set(recording),
        recording_mbid: Set(None),
        recording_isrc: Set(None),
        release_track_mbid: Set(None),
        recording_content_hash: Set(file.content_hash),
        recording_content_size: Set(file.content_size),
        title: Set(title.to_owned()),
        track_title_key: Set(tracks::title_key(title)),
        track_number: Set(track_number),
        disc_number: Set(disc_number),
        duration_ms: Set(file
            .duration
            .to_f64()
            .map(|seconds| (seconds.max(0.0) * 1000.0).round() as i64)),
        import_date: Set(file.last_modified),
        sample_rate: Set(Some(file.sample_rate)),
        ..Default::default()
    })
    .on_conflict(
        OnConflict::column(A::FileId)
            .update_columns([
                A::DiscId,
                A::RecordingId,
                A::RecordingMbid,
                A::RecordingIsrc,
                A::ReleaseTrackMbid,
                A::RecordingContentHash,
                A::RecordingContentSize,
                A::Title,
                A::TrackTitleKey,
                A::TrackNumber,
                A::DiscNumber,
                A::DurationMs,
                A::ImportDate,
                A::SampleRate,
            ])
            .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    native_audio::Entity::delete_by_id(file.id).exec(db).await?;

    // Retain native evidence only while its content identity still matches.
    use album_evidence::Column as E;
    album_evidence::Entity::insert(album_evidence::ActiveModel {
        file_id: Set(file.id),
        title: Set(album_title.to_owned()),
        tag_title: Set(tag_title.map(str::to_owned)),
        title_priority: Set(0),
        version_priority: Set(0),
        ..Default::default()
    })
    .on_conflict(
        OnConflict::column(E::FileId)
            .update_columns([E::Title, E::TagTitle, E::TitlePriority, E::VersionPriority])
            .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    use disc_evidence::Column as D;
    disc_evidence::Entity::insert(disc_evidence::ActiveModel {
        file_id: Set(file.id),
        title_priority: Set(0),
        directory_path: Set(Some(file.directory)),
        ..Default::default()
    })
    .on_conflict(
        OnConflict::column(D::FileId)
            .update_columns([D::TitlePriority, D::DirectoryPath])
            .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    replace_lists(db, file.id, &fields).await?;
    Ok(Some(file.id))
}

async fn replace_lists(
    db: &impl ConnectionTrait,
    file: i32,
    tags: &BTreeMap<&str, Vec<&str>>,
) -> Result<(), DbErr> {
    use credits::Column as C;
    use values::Column as V;
    let audio = CreditScope::Audio as i32;
    credits::Entity::delete_many()
        .filter(C::FileId.eq(file))
        .filter(C::SourceKind.eq("audio_tag"))
        .exec(db)
        .await?;
    values::Entity::delete_many()
        .filter(V::FileId.eq(file))
        .filter(V::Scope.eq(audio))
        .filter(V::Field.eq("genres"))
        .exec(db)
        .await?;

    // Independent provider credits are retained across repeated source projections.
    let names: Vec<_> = [
        (CreditScope::Album, "album_artist"),
        (CreditScope::Audio, "artist"),
    ]
    .into_iter()
    .flat_map(|(scope, key)| {
        tags.get(key)
            .into_iter()
            .flatten()
            .enumerate()
            .map(move |(position, name)| (scope as i32, position as i32, *name))
    })
    .collect();
    for names in names.chunks(CHUNK) {
        credits::Entity::insert_many(names.iter().map(|&(scope, position, name)| {
            credits::ActiveModel {
                file_id: Set(file),
                scope: Set(scope),
                position: Set(position),
                credited_as: Set(name.to_owned()),
                role: Set("Primary Artist".into()),
                source_kind: Set("audio_tag".into()),
                source_provider: Set(String::new()),
                ..Default::default()
            }
        }))
        .exec_without_returning(db)
        .await?;
    }

    // Retain the historical separator for source tags written by older peers.
    let genres: Vec<_> = tags
        .get("genre")
        .into_iter()
        .flatten()
        .flat_map(|value| value.split("\\\\"))
        .filter(|name| !name.is_empty())
        .collect();
    for (batch, genres) in genres.chunks(CHUNK).enumerate() {
        values::Entity::insert_many(genres.iter().enumerate().map(|(position, name)| {
            values::ActiveModel {
                file_id: Set(file),
                scope: Set(audio),
                field: Set("genres".into()),
                qualifier: Set(String::new()),
                position: Set((batch * CHUNK + position) as i32),
                value: Set((*name).to_owned()),
            }
        }))
        .exec_without_returning(db)
        .await?;
    }
    Ok(())
}
