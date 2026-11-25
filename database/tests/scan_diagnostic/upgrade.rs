use std::collections::BTreeSet;

use database::{
    actions::{index::index_audio_library, mixes::query_mix_media_files},
    connection::RecommendationDbConnection,
    entities::{audio_artists, portable_albums, portable_artists, track_album_sources},
    sync::model::{Collection, ObjectKey, Payload, RuneModel},
};
use migration::{
    Migrator, MigratorTrait,
    catalog_identity::{ParentKind, membership, parent},
};
use sea_orm::{
    DatabaseConnection, ExprTrait, QueryOrder, Value,
    sea_query::{Alias, Func, Query},
};

use super::*;

async fn seed(
    db: &DatabaseConnection,
    table: &str,
    fields: &[&str],
    values: Vec<Value>,
) -> Result<()> {
    db.execute(
        Query::insert()
            .into_table(Alias::new(table))
            .columns(fields.iter().map(|field| Alias::new(*field)))
            .values_panic(values.into_iter().map(Expr::val)),
    )
    .await?;
    Ok(())
}

pub(super) async fn assert_recordings(
    db: &DatabaseConnection,
    count: u64,
) -> Result<BTreeSet<String>> {
    assert_recording_counts(db, count, count).await
}

async fn assert_recording_counts(
    db: &DatabaseConnection,
    files: u64,
    performances: u64,
) -> Result<BTreeSet<String>> {
    let audio = Alias::new("music_metadata_audio");
    let tracks = Alias::new("music_metadata_release_tracks");
    let assets = Alias::new("music_metadata_track_assets");
    let field = |table: Alias, name| Expr::col((table, Alias::new(name)));
    let matched = db
        .query_all(
            Query::select()
                .expr(field(audio.clone(), "recording_id"))
                .from(audio.clone())
                .inner_join(
                    assets.clone(),
                    field(audio.clone(), "file_id").eq(field(assets.clone(), "file_id")),
                )
                .inner_join(
                    tracks.clone(),
                    field(tracks.clone(), "id").eq(field(assets, "track_id")),
                )
                .and_where(
                    field(audio.clone(), "recording_id").eq(field(tracks.clone(), "recording_id")),
                ),
        )
        .await?;

    // Include orphan recordings in the count so an unreferenced duplicate fails the check.
    for (table, count) in [
        (audio, files),
        (tracks, performances),
        (Alias::new("music_metadata_recordings"), performances),
    ] {
        let actual: i64 = db
            .query_one(
                Query::select()
                    .expr_as(Func::count(Expr::val(1)), "count")
                    .from(table),
            )
            .await?
            .unwrap()
            .try_get("", "count")?;
        assert_eq!(actual as u64, count);
    }
    assert_eq!(matched.len() as u64, files);
    Ok(matched
        .iter()
        .map(|row| row.try_get("", "recording_id"))
        .collect::<Result<_, _>>()?)
}

#[tokio::test]
async fn whole_file_variants_share_one_recording_after_scan_and_reindex() -> Result<()> {
    let temp = tempfile::tempdir()?;
    for name in ["01 Song.wav", "1 Song.wav"] {
        write_wav_with_info(
            &temp.path().join(name),
            &[(b"INAM", "Song"), (b"IPRD", "Album")],
        )?;
    }
    let actor = Uuid::new_v4();
    let db = fixtures::memory_database(actor).await?;
    let library = fixtures::open_library(db.clone(), actor, None).await?;
    let store = library.record_store();

    // Both whole files occupy one release slot and must leave no unused per-file recording.
    fixtures::scan_library(&FsIo::new(), &db, &actor.to_string(), temp.path(), &store).await?;
    assert_recording_counts(&db, 2, 1).await?;
    index_audio_library(&db, &actor.to_string(), 50, None, &store).await?;
    assert_recording_counts(&db, 2, 1).await?;

    // A retag that separates the release slots must also separate their fallback recordings.
    write_wav_with_info(
        &temp.path().join("01 Song.wav"),
        &[(b"INAM", "Other Song"), (b"IPRD", "Album")],
    )?;
    fixtures::scan_library(&FsIo::new(), &db, &actor.to_string(), temp.path(), &store).await?;
    assert_recordings(&db, 2).await?;
    Ok(())
}

#[tokio::test]
async fn nul_terminated_riff_names_keep_catalog_ids_after_upgrade_and_scan() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let actor = Uuid::new_v4();
    let db = fixtures::memory_database(actor).await?;
    Migrator::down(&db, Some(1)).await?;

    // Include both spellings to check that normalization preserves existing memberships.
    for (file_id, suffix) in [(1, "\0"), (2, "")] {
        let album = format!("Album{suffix}");
        let artist = format!("Beta{suffix}");
        let album_uuid = parent(ParentKind::Album, &album);
        let artist_uuid = parent(ParentKind::Artist, &artist);
        let file_uuid = Uuid::new_v4();
        let name = format!("track-{file_id}.wav");
        write_wav_with_info(
            &temp.path().join(&name),
            &[(b"IART", "Beta"), (b"IPRD", "Album")],
        )?;
        seed(
            &db,
            "albums",
            &["id", "name", "group", "source_uuid"],
            vec![
                file_id.into(),
                album.clone().into(),
                "A".into(),
                album_uuid.to_string().into(),
            ],
        )
        .await?;
        seed(
            &db,
            "artists",
            &["id", "name", "group", "source_uuid"],
            vec![
                file_id.into(),
                artist.clone().into(),
                "B".into(),
                artist_uuid.to_string().into(),
            ],
        )
        .await?;

        // These are the source rows produced by master's INFO reader, including the terminator.
        seed(
            &db,
            "media_files",
            &[
                "id",
                "file_name",
                "directory",
                "extension",
                "file_hash",
                "last_modified",
                "sample_rate",
                "duration",
                "source_uuid",
                "local_available",
                "change_token",
            ],
            vec![
                file_id.into(),
                name.clone().into(),
                "".into(),
                "wav".into(),
                "old-crc".into(),
                "0".into(),
                44100.into(),
                1.into(),
                file_uuid.to_string().into(),
                true.into(),
                "old-token".into(),
            ],
        )
        .await?;
        seed(
            &db,
            "media_file_albums",
            &["id", "track_uuid", "album_uuid", "source_uuid"],
            vec![
                file_id.into(),
                file_uuid.to_string().into(),
                album_uuid.to_string().into(),
                file_uuid.to_string().into(),
            ],
        )
        .await?;
        let link = membership(file_uuid, ParentKind::Artist, artist_uuid);
        seed(
            &db,
            "media_file_artists",
            &["id", "track_uuid", "artist_uuid", "source_uuid"],
            vec![
                file_id.into(),
                file_uuid.to_string().into(),
                artist_uuid.to_string().into(),
                link.to_string().into(),
            ],
        )
        .await?;
        for (position, key, value) in [(0, "album", album), (1, "artist", artist)] {
            seed(
                &db,
                "media_metadata",
                &[
                    "file_id",
                    "position",
                    "meta_key",
                    "meta_value",
                    "source_uuid",
                ],
                vec![
                    file_id.into(),
                    position.into(),
                    key.into(),
                    value.into(),
                    Uuid::new_v4().to_string().into(),
                ],
            )
            .await?;
        }
    }
    Migrator::up(&db, None).await?;
    let before_recordings = assert_recordings(&db, 2).await?;
    let albums = portable_albums::Entity::find()
        .order_by_asc(portable_albums::Column::Id)
        .all(&db)
        .await?;
    let artists = portable_artists::Entity::find()
        .order_by_asc(portable_artists::Column::Id)
        .all(&db)
        .await?;
    let links = audio_artists::Entity::find()
        .order_by_asc(audio_artists::Column::Id)
        .all(&db)
        .await?;

    // Scan twice and reindex through the normal APIs, keeping the imported source IDs.
    let library = fixtures::open_library(db.clone(), actor, None).await?;
    let store = library.record_store();
    for _ in 0..2 {
        fixtures::scan_library(&FsIo::new(), &db, &actor.to_string(), temp.path(), &store).await?;
        index_audio_library(&db, &actor.to_string(), 50, None, &store).await?;
        assert_eq!(
            portable_albums::Entity::find()
                .order_by_asc(portable_albums::Column::Id)
                .all(&db)
                .await?,
            albums
        );
        assert_eq!(
            portable_artists::Entity::find()
                .order_by_asc(portable_artists::Column::Id)
                .all(&db)
                .await?,
            artists
        );
        assert_eq!(
            audio_artists::Entity::find()
                .order_by_asc(audio_artists::Column::Id)
                .all(&db)
                .await?,
            links
        );
        assert_eq!(assert_recordings(&db, 2).await?, before_recordings);
    }

    // Saved numeric Mix selectors still target their original file, including colliding names.
    for file_id in [1, 2] {
        let files = query_mix_media_files(
            &db,
            &RecommendationDbConnection::new(),
            vec![("lib::album".into(), file_id.to_string())],
            0,
            10,
            &[],
        )
        .await?;
        assert_eq!(
            files.iter().map(|file| file.id).collect::<Vec<_>>(),
            [file_id]
        );
        let assigned = track_album_sources::Entity::find()
            .filter(track_album_sources::Column::TrackUuid.eq(&files[0].source_uuid))
            .one(&db)
            .await?
            .unwrap();
        assert_eq!(
            assigned.album_uuid,
            albums[file_id as usize - 1].source_uuid
        );
    }

    // Raw sync spellings retain the hashes peers validate, while the local lookup is normalized.
    for album in albums {
        let key = ObjectKey {
            collection: Collection::Albums,
            id: album.source_uuid.parse()?,
        };
        let Some(Payload::Album(value)) = RuneModel::encode_on(&db, &key).await? else {
            unreachable!()
        };
        assert_eq!(parent(ParentKind::Album, &value.name), key.id);
    }
    Ok(())
}
