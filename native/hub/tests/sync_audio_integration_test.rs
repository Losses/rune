#[path = "support/fixtures.rs"]
mod fixtures;
use anyhow::Result;
use axum::body::Body;
use axum::http::Response;
use database::sync::model::*;
use database::{
    actions::{
        cover_art::assign_inspected,
        downloads::set_download_rule,
        file::is_materialized,
        metadata::get_parsed_file_by_id,
        mixes::{create_mix, update_mix},
        playlists::{add_item_to_playlist, create_playlist, get_all_playlists},
        stats::{get_liked, set_liked},
    },
    connection::RecommendationDbConnection,
    entities::{
        albums, artists, media_cover_art, media_file_artists, media_file_playlists, media_files,
    },
    sync::model::author_source_on,
};
use fsio::FsIo;
use http_request::sync::MetadataPeer;
use hub::server::sync::{
    client::sync_library,
    downloads::download_requested_audio,
    http::{BlobContext, SharedLibrary, blob_router},
    library_router,
};
use migration::catalog_identity::ParentKind;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use uuid::Uuid;

#[tokio::test]
async fn metadata_is_automatic_and_audio_follows_track_artist_album_and_mix_rules() -> Result<()> {
    let source_root = tempfile::tempdir()?;
    let source_node = Uuid::new_v4();
    let source_db = fixtures::initialized_memory(source_node).await?;
    let fsio = Arc::new(FsIo::new());
    for index in 0..2 {
        let fixture = fixtures::audio_fixture_at(index);
        std::fs::copy(
            fixture,
            source_root.path().join(format!("source{index}.ogg")),
        )?;
    }
    let store = fixtures::open_library(source_db.clone(), source_node, None).await?;
    fixtures::scan_library(
        &fsio,
        &source_db,
        &source_node.to_string(),
        source_root.path(),
        &store.record_store(),
    )
    .await?;
    let source_files = media_files::Entity::find().all(&source_db).await?;
    for file in &source_files {
        let index = if file.file_name == "source0.ogg" {
            0
        } else {
            1
        };
        let file_id: Uuid = file.source_uuid.parse()?;
        let artist_id =
            migration::catalog_identity::parent(ParentKind::Artist, &format!("Artist {index}"));
        let album_id =
            migration::catalog_identity::parent(ParentKind::Album, &format!("Album {index}"));
        RuneModel::project(
            &source_db,
            &ObjectKey {
                collection: Collection::Artists,
                id: artist_id,
            },
            Some(&Payload::Artist(Artist {
                name: format!("Artist {index}"),
                group: "A".into(),
            })),
        )
        .await?;
        RuneModel::project(
            &source_db,
            &ObjectKey {
                collection: Collection::Albums,
                id: album_id,
            },
            Some(&Payload::Album(Album {
                name: format!("Album {index}"),
                group: "A".into(),
            })),
        )
        .await?;
        media_file_artists::Entity::delete_many()
            .filter(media_file_artists::Column::TrackUuid.eq(file.source_uuid.clone()))
            .exec(&source_db)
            .await?;
        let membership =
            migration::catalog_identity::membership(file_id, ParentKind::Artist, artist_id);
        RuneModel::project(
            &source_db,
            &ObjectKey {
                collection: Collection::FileArtists,
                id: membership,
            },
            Some(&Payload::FileArtist(FileArtist { file_id, artist_id })),
        )
        .await?;
        RuneModel::project(
            &source_db,
            &ObjectKey {
                collection: Collection::TrackAlbums,
                id: file_id,
            },
            Some(&Payload::TrackAlbum(TrackAlbum {
                track_id: file_id,
                album_id,
                track_number: Some(1),
            })),
        )
        .await?;
        let track_key = ObjectKey {
            collection: Collection::Tracks,
            id: file_id,
        };
        let Some(Payload::Track(mut track)) = RuneModel::encode_on(&source_db, &track_key).await?
        else {
            anyhow::bail!("track missing");
        };
        if let Some(title) = track
            .metadata
            .iter_mut()
            .find(|tag| tag.key == "track_title")
        {
            title.value = format!("Recording {index}");
        } else {
            track.metadata.push(MetadataTag {
                id: Uuid::new_v4(),
                position: track.metadata.len() as i32,
                key: "track_title".into(),
                value: format!("Recording {index}"),
            });
        }
        RuneModel::project(&source_db, &track_key, Some(&Payload::Track(track))).await?;
        // These projected fixture edits replace rows already authored by the scan.
        use sea_orm::TransactionTrait;
        let tx = source_db.begin().await?;
        store.record_store().lock(&tx).await?;
        for key in [
            track_key,
            ObjectKey {
                collection: Collection::TrackAlbums,
                id: file_id,
            },
        ] {
            author_source_on(&tx, &store.record_store(), key).await?;
        }
        tx.commit().await?;
    }
    let first = source_files
        .iter()
        .find(|f| f.file_name == "source0.ogg")
        .unwrap();

    let art_body = b"verified artwork body";
    let digest = blake3::hash(art_body).to_hex().to_string();
    let art_id = migration::catalog_identity::artwork(&digest);
    assign_inspected(
        &source_db,
        &store.record_store(),
        first,
        Some(metadata::cover_art::CoverArt {
            crc: "legacy-crc".into(),
            data: art_body.to_vec(),
            primary_color: 42,
        }),
    )
    .await?;
    set_liked(&source_db, &store.record_store(), first.id, true).await?;
    let playlist = create_playlist(
        &source_db,
        &store.record_store(),
        "Selected music".into(),
        "S".into(),
    )
    .await?;
    for _ in 0..2 {
        add_item_to_playlist(
            &source_db,
            &store.record_store(),
            playlist.id,
            first.id,
            None,
        )
        .await?;
    }

    store.import_existing(source_node).await?;
    let shared = Arc::new(RwLock::new(store.clone()));
    let transfers = Arc::new(AtomicUsize::new(0));
    let counter = transfers.clone();
    let corrupt_art = Arc::new(AtomicBool::new(false));
    let corrupt_art_handler = corrupt_art.clone();
    let app = library_router(SharedLibrary(shared.clone()), Arc::new(|| {}))
        .merge(blob_router(BlobContext {
            store: shared,
            db: source_db.clone(),
            fsio: fsio.clone(),
            root: source_root.path().into(),
            node_id: source_node,
            upload_lock: Arc::new(Mutex::new(())),
            notify: Arc::new(|| {}),
        }))
        .layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let counter = counter.clone();
                let corrupt_art_handler = corrupt_art_handler.clone();
                async move {
                    if request.uri().path().starts_with("/sync/artworks/")
                        && corrupt_art_handler.load(Ordering::SeqCst)
                    {
                        return Response::builder()
                            .status(200)
                            .body(Body::from("corrupt"))
                            .unwrap();
                    }
                    if request.uri().path().starts_with("/sync/blobs/") {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                    next.run(request).await
                }
            },
        ));
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let recommendation = RecommendationDbConnection::new();

    for kind in ["track", "artist", "album", "mix"] {
        let root = tempfile::tempdir()?;
        let node = Uuid::new_v4();
        let db = fixtures::initialized_memory(node).await?;
        let before = transfers.load(Ordering::SeqCst);
        if kind == "track" {
            corrupt_art.store(true, Ordering::SeqCst);
        }
        let first_report = sync_library(db.clone(), node, &url, reqwest::Client::new()).await?;
        if kind == "track" {
            assert!(
                first_report.artwork_pending > 0,
                "corrupt artwork must remain pending"
            );
            assert_eq!(
                media_files::Entity::find().all(&db).await?.len(),
                2,
                "corrupt artwork must not block metadata"
            );
            assert!(
                media_cover_art::Entity::find()
                    .filter(media_cover_art::Column::LocalAvailable.eq(true))
                    .all(&db)
                    .await?
                    .is_empty()
            );
            corrupt_art.store(false, Ordering::SeqCst);
            sync_library(db.clone(), node, &url, reqwest::Client::new()).await?;
        }
        assert_eq!(
            transfers.load(Ordering::SeqCst),
            before,
            "metadata must not even probe audio"
        );
        assert_eq!(std::fs::read_dir(root.path())?.count(), 0);
        let catalog = media_files::Entity::find().all(&db).await?;
        assert_eq!(catalog.len(), 2);
        let local = fixtures::open_library(db.clone(), node, None).await?;
        let id = media_files::Entity::find()
            .filter(media_files::Column::SourceUuid.eq(first.source_uuid.clone()))
            .one(&db)
            .await?
            .unwrap()
            .id;
        let artist = artists::Entity::find()
            .filter(artists::Column::Name.eq("Artist 0"))
            .one(&db)
            .await?
            .unwrap();
        let album = albums::Entity::find()
            .filter(albums::Column::Name.eq("Album 0"))
            .one(&db)
            .await?
            .unwrap();
        let received_art = media_cover_art::Entity::find()
            .filter(media_cover_art::Column::SourceUuid.eq(art_id.to_string()))
            .one(&db)
            .await?
            .expect("artwork metadata missing");
        assert!(received_art.local_available);
        assert_eq!(received_art.binary, art_body);

        let other = catalog.iter().find(|f| f.id != id).unwrap().id;
        assert!(!is_materialized(&db, &fsio, root.path(), id).await?);
        assert!(get_liked(&db, id).await?);
        assert_eq!(get_all_playlists(&db).await?.len(), 1);
        assert_eq!(
            media_file_playlists::Entity::find().all(&db).await?.len(),
            2
        );
        let (queries, mix) = match kind {
            "mix" => {
                let mix = create_mix(
                    &db,
                    &local.record_store(),
                    "Offline mix".into(),
                    "O".into(),
                    false,
                    0,
                    false,
                    vec![("lib::track".into(), id.to_string())],
                )
                .await?;
                (vec![], Some(mix))
            }
            _ => {
                let selected = match kind {
                    "artist" => artist.id,
                    "album" => album.id,
                    _ => id,
                };
                (vec![(format!("lib::{kind}"), selected.to_string())], None)
            }
        };
        set_download_rule(&db, queries.clone(), mix.as_ref().map(|m| m.id), true).await?;
        // Saving a mix is an intentional metadata edit, separate from downloading it.
        sync_library(db.clone(), node, &url, reqwest::Client::new()).await?;
        let fetched = download_requested_audio(
            db.clone(),
            &recommendation,
            fsio.clone(),
            root.path().into(),
            node,
            &MetadataPeer::new(&url, reqwest::Client::new()),
            Arc::new(|| {}),
        )
        .await?;
        assert_eq!(
            fetched.downloaded, 1,
            "{kind} should fetch exactly one selected file"
        );
        assert!(is_materialized(&db, &fsio, root.path(), id).await?);
        assert!(!is_materialized(&db, &fsio, root.path(), other).await?);
        let materialized = media_files::Entity::find_by_id(id).one(&db).await?.unwrap();
        let expected = fixtures::audio_fixture();
        assert_eq!(
            std::fs::read(
                root.path()
                    .join(&materialized.directory)
                    .join(&materialized.file_name)
            )?,
            std::fs::read(expected)?
        );
        assert_eq!(
            media_files::Entity::find().all(&db).await?.len(),
            2,
            "download keeps the catalog ID"
        );
        get_parsed_file_by_id(&db, id).await?;
        assert!(get_liked(&db, id).await?);
        let head = store.head().await?;
        sync_library(db.clone(), node, &url, reqwest::Client::new()).await?;
        let again = download_requested_audio(
            db.clone(),
            &recommendation,
            fsio.clone(),
            root.path().into(),
            node,
            &MetadataPeer::new(&url, reqwest::Client::new()),
            Arc::new(|| {}),
        )
        .await?;
        assert_eq!(again.downloaded, 0);
        assert_eq!(transfers.load(Ordering::SeqCst), before + 1);
        assert_eq!(
            store.head().await?,
            head,
            "materialization must not create metadata feedback"
        );
        if let Some(mix) = &mix {
            update_mix(
                &db,
                &local.record_store(),
                mix.id,
                Some(mix.name.clone()),
                Some(mix.group.clone()),
                Some(mix.scriptlet_mode),
                Some(mix.mode.unwrap_or(0)),
                Some(mix.locked),
                Some(vec![("lib::track".into(), other.to_string())]),
            )
            .await?;
            let changed = download_requested_audio(
                db.clone(),
                &recommendation,
                fsio.clone(),
                root.path().into(),
                node,
                &MetadataPeer::new(&url, reqwest::Client::new()),
                Arc::new(|| {}),
            )
            .await?;
            assert_eq!(
                changed.downloaded, 1,
                "a saved mix rule follows edits to its definition"
            );
            assert!(is_materialized(&db, &fsio, root.path(), other).await?);
        }
        set_download_rule(&db, queries, mix.as_ref().map(|m| m.id), false).await?;
        let disabled = download_requested_audio(
            db.clone(),
            &recommendation,
            fsio.clone(),
            root.path().into(),
            node,
            &fixtures::unreachable_peer(),
            Arc::new(|| {}),
        )
        .await?;
        assert_eq!(disabled.requested, 0);
        assert!(
            is_materialized(&db, &fsio, root.path(), id).await?,
            "disabling retention does not delete audio"
        );
    }
    server.abort();
    Ok(())
}
