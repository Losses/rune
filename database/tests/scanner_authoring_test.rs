#[path = "support/fixtures.rs"]
mod fixtures;
use anyhow::Result;
use database::{
    actions::{
        index::{index_audio_library, index_media_files},
        metadata::{author_indexed_track_row, sync_file_descriptions},
    },
    entities::{albums, media_file_albums, media_file_artists, media_file_genres, media_files},
    sync::{
        SeaOrmSyncStore,
        model::{Collection, ObjectKey},
    },
};
use fsio::FsIo;
use metadata::describe::describe_file;
use sea_orm::{ConnectionTrait, Database, EntityTrait, PaginatorTrait};
use uuid::Uuid;

#[tokio::test]
async fn journal_failure_rolls_back_scanned_file() -> Result<()> {
    let node = Uuid::from_u128(1);
    let db = fixtures::memory_database(node).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::from_u128(2), node).await?;
    let fs = FsIo::new();
    let root = fs.canonicalize(&fixtures::asset_directory())?.path;
    let audio = fs.canonicalize(&root.join("startup_0.ogg"))?;
    let mut descriptions = vec![Some(describe_file(&fs, &audio, &Some(root))?)];
    db.execute_unprepared("CREATE TRIGGER reject_journal BEFORE INSERT ON sync_journal BEGIN SELECT RAISE(ABORT, 'test failure'); END").await?;
    assert!(
        sync_file_descriptions(
            &fs,
            &db,
            &node.to_string(),
            &mut descriptions,
            false,
            &store
        )
        .await
        .is_err()
    );
    assert_eq!(media_files::Entity::find().count(&db).await?, 0);
    db.execute_unprepared("DROP TRIGGER reject_journal").await?;
    sync_file_descriptions(
        &fs,
        &db,
        &node.to_string(),
        &mut descriptions,
        false,
        &store,
    )
    .await?;
    assert_eq!(media_files::Entity::find().count(&db).await?, 1);
    assert_eq!(
        sync_orm::causal_progress::head_on(&db, store.namespace())
            .await?
            .sequence,
        1
    );
    let file = media_files::Entity::find().one(&db).await?.unwrap();
    db.execute_unprepared("CREATE TRIGGER reject_album_journal BEFORE INSERT ON sync_journal BEGIN SELECT RAISE(ABORT, 'test failure'); END").await?;
    assert!(
        index_audio_library(&db, &node.to_string(), 50, None, &store)
            .await
            .is_err()
    );
    assert_eq!(albums::Entity::find().count(&db).await?, 0);
    assert_eq!(media_file_albums::Entity::find().count(&db).await?, 0);
    db.execute_unprepared("DROP TRIGGER reject_album_journal")
        .await?;
    index_audio_library(&db, &node.to_string(), 50, None, &store).await?;
    let indexed_count = sync_orm::causal_progress::head_on(&db, store.namespace())
        .await?
        .sequence;
    assert!(indexed_count >= 3);
    assert!(
        store
            .get_on(
                &db,
                &ObjectKey {
                    collection: Collection::TrackAlbums,
                    id: file.source_uuid.parse()?
                }
            )
            .await?
            .is_some()
    );
    let artists_before = media_file_artists::Entity::find().all(&db).await?;
    let genres_before = media_file_genres::Entity::find().all(&db).await?;
    sync_file_descriptions(&fs, &db, &node.to_string(), &mut descriptions, true, &store).await?;
    index_media_files(&db, &node.to_string(), vec![file.id], None, &store).await?;
    assert_eq!(
        media_file_artists::Entity::find().all(&db).await?,
        artists_before
    );
    assert_eq!(
        media_file_genres::Entity::find().all(&db).await?,
        genres_before
    );
    assert_eq!(
        sync_orm::causal_progress::head_on(&db, store.namespace())
            .await?
            .sequence,
        indexed_count,
        "forced rescanning and indexing unchanged metadata must not author changes"
    );

    Ok(())
}

#[tokio::test]
async fn index_reads_metadata_after_acquiring_replica_lock() -> Result<()> {
    use database::actions::{index::index_media_files, metadata::scan_audio_file};
    use database::entities::{
        albums, artists, genres, media_file_albums, media_file_artists, media_file_genres,
        media_metadata,
    };
    use database::sync::model::Payload;
    use sea_orm::{ColumnTrait, QueryFilter, Set};
    let db = Database::connect("sqlite::memory:").await?;
    let actor = Uuid::new_v4();
    migration::up_with_actor(&db, Some(&actor.to_string()), None).await?;
    let library = fixtures::open_library(db.clone(), actor, None).await?;
    let store = library.record_store();
    let root = FsIo::new().canonicalize(&fixtures::asset_directory())?.path;
    let file_id = scan_audio_file(
        &FsIo::new(),
        &db,
        &actor.to_string(),
        &root,
        &root.join("startup_0.ogg"),
        &store,
    )
    .await?;
    media_metadata::Entity::delete_many()
        .filter(media_metadata::Column::FileId.eq(file_id))
        .exec(&db)
        .await?;
    for (position, key) in ["artist", "album", "genre"].into_iter().enumerate() {
        media_metadata::Entity::insert(media_metadata::ActiveModel {
            file_id: Set(file_id),
            meta_key: Set(key.into()),
            meta_value: Set(format!("old {key}")),
            source_uuid: Set(Uuid::new_v4().to_string()),
            position: Set(position as i32),
            ..Default::default()
        })
        .exec(&db)
        .await?;
    }
    // SQLite triggers inject a deterministic source change at lock acquisition.
    // The trigger cannot be represented by SeaORM entity operations.
    db.execute_unprepared("CREATE TRIGGER change_tags_at_replica_lock AFTER UPDATE OF head ON sync_meta WHEN NEW.head = OLD.head BEGIN UPDATE media_metadata SET meta_value = CASE meta_key WHEN 'artist' THEN 'new artist' WHEN 'album' THEN 'new album' WHEN 'genre' THEN 'new genre' ELSE meta_value END; END").await?;
    index_media_files(&db, &actor.to_string(), vec![file_id], None, &store).await?;
    db.execute_unprepared("DROP TRIGGER change_tags_at_replica_lock")
        .await?;
    let file = media_files::Entity::find_by_id(file_id)
        .one(&db)
        .await?
        .unwrap();
    let artist_ids: Vec<_> = media_file_artists::Entity::find()
        .filter(media_file_artists::Column::TrackUuid.eq(&file.source_uuid))
        .all(&db)
        .await?
        .into_iter()
        .map(|row| row.artist_uuid)
        .collect();
    let album_ids: Vec<_> = media_file_albums::Entity::find()
        .filter(media_file_albums::Column::TrackUuid.eq(&file.source_uuid))
        .all(&db)
        .await?
        .into_iter()
        .map(|row| row.album_uuid)
        .collect();
    let genre_ids: Vec<_> = media_file_genres::Entity::find()
        .filter(media_file_genres::Column::TrackUuid.eq(&file.source_uuid))
        .all(&db)
        .await?
        .into_iter()
        .map(|row| row.genre_uuid)
        .collect();
    assert_eq!(
        artists::Entity::find()
            .filter(artists::Column::SourceUuid.is_in(artist_ids))
            .all(&db)
            .await?
            .into_iter()
            .map(|row| row.name)
            .collect::<Vec<_>>(),
        vec!["new artist"]
    );
    assert_eq!(
        albums::Entity::find()
            .filter(albums::Column::SourceUuid.is_in(album_ids))
            .all(&db)
            .await?
            .into_iter()
            .map(|row| row.name)
            .collect::<Vec<_>>(),
        vec!["new album"]
    );
    assert_eq!(
        genres::Entity::find()
            .filter(genres::Column::SourceUuid.is_in(genre_ids))
            .all(&db)
            .await?
            .into_iter()
            .map(|row| row.name)
            .collect::<Vec<_>>(),
        vec!["NEW GENRE"]
    );
    let file = media_files::Entity::find_by_id(file_id)
        .one(&db)
        .await?
        .unwrap();
    assert!(
        artists::Entity::find()
            .filter(artists::Column::Name.eq("old artist"))
            .one(&db)
            .await?
            .is_none()
    );
    assert!(
        albums::Entity::find()
            .filter(albums::Column::Name.eq("old album"))
            .one(&db)
            .await?
            .is_none()
    );
    assert!(
        genres::Entity::find()
            .filter(genres::Column::Name.eq("OLD GENRE"))
            .one(&db)
            .await?
            .is_none()
    );
    let record = library
        .get(&ObjectKey {
            collection: Collection::Tracks,
            id: file.source_uuid.parse()?,
        })
        .await?
        .unwrap();
    let Some(Payload::Track(track)) = record.value else {
        panic!("missing track assignment")
    };
    for key in ["artist", "album", "genre"] {
        assert!(
            track
                .metadata
                .iter()
                .any(|tag| tag.key == key && tag.value == format!("new {key}"))
        );
    }
    Ok(())
}

#[tokio::test]
async fn untitled_track_is_searchable_after_authoring() -> Result<()> {
    use database::actions::collection::CollectionQueryType;
    use database::{
        actions::{metadata::scan_audio_file, search::search_for},
        entities::media_metadata,
    };
    use sea_orm::{ColumnTrait, QueryFilter, TransactionTrait};
    let node = Uuid::new_v4();
    let db = fixtures::memory_database(node).await?;
    let store = SeaOrmSyncStore::create_empty(db.clone(), Uuid::new_v4(), node).await?;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("unusualfallback.ogg");
    std::fs::copy(fixtures::audio_fixture(), &path)?;
    let id = scan_audio_file(
        &FsIo::new(),
        &db,
        &node.to_string(),
        dir.path(),
        &path,
        &store,
    )
    .await?;
    let tx = db.begin().await?;
    store.lock(&tx).await?;
    media_metadata::Entity::delete_many()
        .filter(media_metadata::Column::FileId.eq(id))
        .filter(media_metadata::Column::MetaKey.eq("track_title"))
        .exec(&tx)
        .await?;
    let file = media_files::Entity::find_by_id(id).one(&tx).await?.unwrap();
    author_indexed_track_row(&tx, &store, file).await?;
    tx.commit().await?;
    let found = search_for(
        &db,
        "unusualfallback",
        Some(vec![CollectionQueryType::Track]),
        30,
    )
    .await?;
    assert!(
        found
            .get(&CollectionQueryType::Track)
            .is_some_and(|ids| ids.contains(&i64::from(id)))
    );
    Ok(())
}
