#[path = "support/causal_library.rs"]
mod support;

use anyhow::Result;
use database::entities::{
    albums, artists, genres, media_file_albums, media_file_artists, media_file_genres,
    media_file_playlists, media_files, playlists,
};
use database::sync::{LibrarySyncStore, model::*};
use migration::catalog_identity::{self, ParentKind};
use sea_orm::{EntityTrait, PaginatorTrait};
use std::{
    collections::BTreeMap,
    env::consts::{ARCH, OS},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

// This diagnostic stores only bounded categories, never SQL text or values.
const METRIC_TABLES: &[&str] = &[
    "sync_context_intervals",
    "sync_context_ranges",
    "sync_candidates",
    "sync_manifests",
    "sync_manifest_edges",
    "sync_chunks",
    "sync_leases",
    "sync_meta",
    "sync_baseline_segments",
    "sync_journal",
    "sync_writers",
    "sync_checkpoint",
    "media_files",
    "media_file_artists",
    "media_file_genres",
    "media_file_albums",
    "media_file_playlists",
    "albums",
    "artists",
    "genres",
    "playlists",
    "media_metadata",
];

#[derive(Default)]
struct QueryTotal {
    count: usize,
    elapsed: Duration,
    failed: usize,
}

type QueryGroups = BTreeMap<(u8, &'static str, &'static str, &'static str), QueryTotal>;

fn instrument(
    db: &sea_orm::DatabaseConnection,
    side: &'static str,
    phase: Arc<AtomicU8>,
    totals: Arc<Mutex<QueryGroups>>,
) -> sea_orm::DatabaseConnection {
    let mut measured = db.clone();
    measured.set_metric_callback(move |info| {
        let current = phase.load(Ordering::Relaxed);
        if current == 0 {
            return;
        }
        let sql = info.statement.sql.to_ascii_lowercase();
        let operation = match sql.split_whitespace().next() {
            Some("select") => "select",
            Some("insert") => "insert",
            Some("update") => "update",
            Some("delete") => "delete",
            _ => "other",
        };
        let table = METRIC_TABLES
            .iter()
            .copied()
            .find(|table| sql.contains(table))
            .unwrap_or("other");
        let mut totals = totals.lock().unwrap();
        let entry = totals.entry((current, side, operation, table)).or_default();
        entry.count += 1;
        entry.elapsed += info.elapsed;
        entry.failed += usize::from(info.failed);
    });
    measured
}

#[tokio::test]
#[ignore = "manual catalog scale measurement"]
async fn catalog_scale() -> Result<()> {
    let count = std::env::var("SYNC_SCALE_TRACKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000usize);
    let lib = Uuid::new_v4();
    let source_actor = Uuid::new_v4();
    let target_actor = Uuid::new_v4();
    let source_sql = support::SqlLibrary::new(lib, source_actor).await?;
    let target_sql = support::SqlLibrary::new(lib, target_actor).await?;
    let phase = Arc::new(AtomicU8::new(0));
    let totals = Arc::new(Mutex::new(QueryGroups::new()));
    let source_db = instrument(&source_sql.db, "source", phase.clone(), totals.clone());
    let target_db = instrument(&target_sql.db, "target", phase.clone(), totals.clone());
    let source_library = LibrarySyncStore::open(source_db, source_actor, Some(lib)).await?;
    let dest = LibrarySyncStore::open(target_db, target_actor, Some(lib)).await?;
    let source: support::Peer = Arc::new(source_library.endpoint());
    let db = target_sql.db.clone();
    phase.store(1, Ordering::Relaxed);
    let preparation = Instant::now();
    let albums: Vec<_> = (0..20)
        .map(|i| catalog_identity::parent(ParentKind::Album, &format!("Album {i}")))
        .collect();
    let artists: Vec<_> = (0..40)
        .map(|i| catalog_identity::parent(ParentKind::Artist, &format!("Artist {i}")))
        .collect();
    let genres: Vec<_> = (0..8)
        .map(|i| catalog_identity::parent(ParentKind::Genre, &format!("Genre {i}")))
        .collect();
    let playlists: Vec<_> = (0..10).map(|_| Uuid::new_v4()).collect();
    for (i, id) in albums.iter().enumerate() {
        source
            .write_local(
                ObjectKey {
                    collection: Collection::Albums,
                    id: *id,
                },
                Some(Payload::Album(Album {
                    name: format!("Album {i}"),
                    group: "A".into(),
                })),
            )
            .await?;
    }
    for (i, id) in artists.iter().enumerate() {
        source
            .write_local(
                ObjectKey {
                    collection: Collection::Artists,
                    id: *id,
                },
                Some(Payload::Artist(Artist {
                    name: format!("Artist {i}"),
                    group: "A".into(),
                })),
            )
            .await?;
    }
    for (i, id) in genres.iter().enumerate() {
        source
            .write_local(
                ObjectKey {
                    collection: Collection::Genres,
                    id: *id,
                },
                Some(Payload::Genre(Genre {
                    name: format!("Genre {i}"),
                    group: "G".into(),
                })),
            )
            .await?;
    }
    for (i, id) in playlists.iter().enumerate() {
        source
            .write_local(
                ObjectKey {
                    collection: Collection::Playlists,
                    id: *id,
                },
                Some(Payload::Playlist(Playlist {
                    name: format!("Playlist {i}"),
                    group: "P".into(),
                })),
            )
            .await?;
    }
    for i in 0..count {
        let track = Uuid::new_v4();

        source
            .write_local(
                ObjectKey {
                    collection: Collection::Tracks,
                    id: track,
                },
                Some(Payload::Track(Track {
                    extension: "ogg".into(),
                    content_hash: Some(format!("{i:064x}")),
                    content_size: Some(1234),
                    duration: "180".to_owned().try_into().map_err(anyhow::Error::msg)?,
                    sample_rate: 44100,
                    cover_art_id: None,
                    metadata: vec![MetadataTag {
                        id: Uuid::new_v4(),
                        position: 0,
                        key: "track_title".into(),
                        value: format!("Track {i}"),
                    }],
                })),
            )
            .await?;
        source
            .write_local(
                ObjectKey::track_album(track),
                Some(Payload::TrackAlbum(TrackAlbum {
                    track_id: track,
                    album_id: albums[i % albums.len()],
                    track_number: Some((i % 20 + 1) as i32),
                })),
            )
            .await?;
        let artist_id = artists[i % artists.len()];
        let genre_id = genres[i % genres.len()];
        source
            .write_local(
                ObjectKey {
                    collection: Collection::FileArtists,
                    id: catalog_identity::membership(track, ParentKind::Artist, artist_id),
                },
                Some(Payload::FileArtist(FileArtist {
                    file_id: track,
                    artist_id,
                })),
            )
            .await?;
        source
            .write_local(
                ObjectKey {
                    collection: Collection::FileGenres,
                    id: catalog_identity::membership(track, ParentKind::Genre, genre_id),
                },
                Some(Payload::FileGenre(FileGenre {
                    file_id: track,
                    genre_id,
                })),
            )
            .await?;
        if i % 10 == 0 {
            source
                .write_local(
                    ObjectKey {
                        collection: Collection::PlaylistEntries,
                        id: Uuid::new_v4(),
                    },
                    Some(Payload::PlaylistEntry(PlaylistEntry {
                        playlist_id: playlists[i % playlists.len()],
                        track_id: track,
                        position: i as i32,
                    })),
                )
                .await?;
        }
    }
    let scope = Scope {
        collections: [
            Collection::Tracks,
            Collection::Albums,
            Collection::Artists,
            Collection::Genres,
            Collection::FileArtists,
            Collection::FileGenres,
            Collection::TrackAlbums,
            Collection::Playlists,
            Collection::PlaylistEntries,
            Collection::Likes,
            Collection::Mixes,
            Collection::PlaybackStatistics,
        ]
        .into_iter()
        .collect(),
    };
    let target: support::Peer = Arc::new(dest.endpoint());
    let source_records = source.head().await?.sequence;
    eprintln!(
        "environment={} {} profile={} storage=sqlite::memory: source_preparation_ms={} tracks={count} source_records={source_records}",
        OS,
        ARCH,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        preparation.elapsed().as_millis()
    );
    phase.store(2, Ordering::Relaxed);
    let start = Instant::now();
    let steps = exchange(&source, &target, scope.clone(), &source_sql.db, &db).await?;
    phase.store(0, Ordering::Relaxed);
    assert_eq!(media_files::Entity::find().count(&db).await?, count as u64);
    assert_eq!(albums::Entity::find().count(&db).await?, 20);
    assert_eq!(artists::Entity::find().count(&db).await?, 40);
    assert_eq!(genres::Entity::find().count(&db).await?, 8);
    assert_eq!(
        media_file_artists::Entity::find().count(&db).await?,
        count as u64
    );
    assert_eq!(
        media_file_genres::Entity::find().count(&db).await?,
        count as u64
    );
    assert_eq!(
        media_file_albums::Entity::find().count(&db).await?,
        count as u64
    );
    assert_eq!(playlists::Entity::find().count(&db).await?, 10);
    assert_eq!(
        media_file_playlists::Entity::find().count(&db).await?,
        count.div_ceil(10) as u64
    );
    println!(
        "tracks={count} albums=20 artists=40 genres=8 playlists=10 entries={} source_records={} initial_coordinator_steps={} initial_ms={}",
        count.div_ceil(10),
        source_records,
        steps,
        start.elapsed().as_millis()
    );
    for i in 1..=3 {
        phase.store(3, Ordering::Relaxed);
        let start = Instant::now();
        let head = dest.head().await?;
        let steps = exchange(&source, &target, scope.clone(), &source_sql.db, &db).await?;
        assert_eq!(dest.head().await?, head);
        phase.store(0, Ordering::Relaxed);
        println!(
            "idle_round={i} head={} coordinator_steps={} elapsed_ms={}",
            head.sequence,
            steps,
            start.elapsed().as_millis()
        );
    }
    for ((phase, side, operation, table), value) in totals.lock().unwrap().iter() {
        let phase = match phase {
            1 => "preparation",
            2 => "initial",
            3 => "idle",
            _ => unreachable!(),
        };
        eprintln!(
            "query phase={phase} side={side} operation={operation} table={table} count={} elapsed_us={} failed={}",
            value.count,
            value.elapsed.as_micros(),
            value.failed
        );
    }
    Ok(())
}

async fn exchange(
    source: &support::Peer,
    target: &support::Peer,
    scope: Scope,
    source_db: &sea_orm::DatabaseConnection,
    target_db: &sea_orm::DatabaseConnection,
) -> Result<usize> {
    tokio::time::timeout(Duration::from_secs(900), async {
        let remote = sync::StorePeer(source.clone());
        let mut coordinator = sync::Coordinator::new(scope, database::sync::session_limits())?;
        let mut steps = 0;
        let started = Instant::now();
        loop {
            steps += 1;
            if steps % 1000 == 0 {
                eprintln!(
                    "transfer_steps={steps} elapsed_ms={}",
                    started.elapsed().as_millis()
                );
                trace_builds("source", source_db).await?;
                trace_builds("target", target_db).await?;
            }
            match coordinator
                .advance(&**target, &**target, &**target, &remote, &**source)
                .await?
            {
                sync::SessionAdvance::Complete(_) => return Ok(steps),
                sync::SessionAdvance::BudgetExhausted => coordinator.start_pass(),
                sync::SessionAdvance::Incomplete => (),
                sync::SessionAdvance::Cancelled => anyhow::bail!("catalog session cancelled"),
            }
        }
    })
    .await?
}

async fn trace_builds(label: &str, db: &sea_orm::DatabaseConnection) -> Result<()> {
    use sea_orm::{
        ConnectionTrait, ExprTrait,
        sea_query::{Alias, Expr, Query},
    };
    let rows = db
        .query_all(
            &Query::select()
                .column(Alias::new("build_state"))
                .from(Alias::new("sync_leases"))
                .and_where(Expr::col(Alias::new("status")).eq(0))
                .limit(16)
                .to_owned(),
        )
        .await?;
    for row in rows {
        let Some(bytes) = row.try_get_by_index::<Option<Vec<u8>>>(0)? else {
            continue;
        };
        let state: serde_json::Value = serde_json::from_slice(&bytes)?;
        eprintln!(
            "{label}_build kind={} phase={} position={} collection={} partition={} context_union={} root_promoted={} summaries={} done={}",
            state["kind"],
            state["phase"],
            state["cursor"]["position"],
            state["collection_index"],
            state["partition_index"],
            state["union"]["phase"],
            state["root_promoted"],
            state["summary_count"],
            state["descriptor_complete"]
        );
    }
    Ok(())
}
