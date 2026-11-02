#[path = "support/causal_library.rs"]
mod support;
use anyhow::Result;
use axum::{
    body::{Body, to_bytes},
    extract::Request,
    middleware::Next,
    response::Response,
};
use database::entities::{media_files, media_metadata};
use http_request::sync::router;
use hub::server::sync::client::sync_library;
use sea_orm::{EntityTrait, PaginatorTrait, Set, TransactionTrait};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};
use support::SqlLibrary;
use uuid::Uuid;
#[derive(Default)]
struct Traffic {
    requests: AtomicUsize,
    bytes: AtomicUsize,
    pages: AtomicUsize,
    blobs: AtomicUsize,
    operations: std::sync::Mutex<std::collections::BTreeMap<String, usize>>,
}
impl Traffic {
    fn reset(&self) {
        self.operations.lock().unwrap().clear();
        for field in [&self.requests, &self.bytes, &self.pages, &self.blobs] {
            field.store(0, Ordering::Relaxed);
        }
    }
}
#[tokio::test]
#[ignore = "manual 10,000-track initial and idle HTTP measurement"]
async fn metadata_initial_incremental_and_idle_rounds() -> Result<()> {
    let count: i32 = std::env::var("SYNC_HTTP_TRACKS")
        .ok()
        .map(|v| v.parse())
        .transpose()?
        .unwrap_or(10000);
    anyhow::ensure!(count > 0, "track count must be positive");
    let ns = Uuid::new_v4();
    let source = SqlLibrary::new(ns, Uuid::new_v4()).await?;
    let actor = Uuid::new_v4();
    let target = SqlLibrary::new(ns, actor).await?;
    let preparation = Instant::now();
    for base in (0..count).step_by(128) {
        let end = (base + 128).min(count);
        let tx = source.db.begin().await?;
        media_files::Entity::insert_many((base..end).map(|index| media_files::ActiveModel {
            id: Set(index + 1),
            hlc_uuid: Set(Uuid::from_u128(index as u128 + 1).to_string()),
            file_name: Set(String::new()),
            directory: Set(String::new()),
            extension: Set("ogg".into()),
            file_hash: Set(String::new()),
            last_modified: Set(String::new()),
            sample_rate: Set(44100),
            duration: Set(180.into()),
            local_available: Set(false),
            content_hash: Set(Some(format!("{index:064x}"))),
            content_size: Set(Some(100)),
            ..Default::default()
        }))
        .exec(&tx)
        .await?;
        media_metadata::Entity::insert_many((base..end).map(|index| media_metadata::ActiveModel {
            file_id: Set(index + 1),
            hlc_uuid: Set(Uuid::new_v4().to_string()),
            position: Set(0),
            meta_key: Set("track_title".into()),
            meta_value: Set(format!("Track {index}")),
            ..Default::default()
        }))
        .exec(&tx)
        .await?;
        tx.commit().await?;
    }
    source.library.import_basic().await?;
    eprintln!(
        "environment={} {} profile={} storage=sqlite::memory: tracks={count} metadata_tags={count} preparation_ms={}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        preparation.elapsed().as_millis()
    );
    let traffic = Arc::new(Traffic::default());
    let observed = traffic.clone();
    let app = router(source.peer.clone()).layer(axum::middleware::from_fn(
        move |request: Request, next: Next| {
            let traffic = observed.clone();
            async move {
                traffic.requests.fetch_add(1, Ordering::Relaxed);
                if request.uri().path().starts_with("/sync/blobs/") {
                    traffic.blobs.fetch_add(1, Ordering::Relaxed);
                }
                let (parts, body) = request.into_parts();
                let bytes = to_bytes(body, 9 * 1024 * 1024).await.unwrap();
                traffic.bytes.fetch_add(bytes.len(), Ordering::Relaxed);
                if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                    if let Some(operation) = value["operation"].as_str() {
                        *traffic
                            .operations
                            .lock()
                            .unwrap()
                            .entry(operation.to_owned())
                            .or_default() += 1;
                    }
                    if matches!(
                        value["operation"].as_str(),
                        Some("read_page" | "apply_page")
                    ) {
                        traffic.pages.fetch_add(1, Ordering::Relaxed);
                    }
                }
                let response = next
                    .run(Request::from_parts(parts, Body::from(bytes)))
                    .await;
                let (parts, body) = response.into_parts();
                let bytes = to_bytes(body, 32 * 1024 * 1024).await.unwrap();
                traffic.bytes.fetch_add(bytes.len(), Ordering::Relaxed);
                Response::from_parts(parts, Body::from(bytes))
            }
        },
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::new();
    let synchronize = || sync_library(target.db.clone(), actor, &base, client.clone());
    let initial = Instant::now();
    synchronize().await?;
    assert_eq!(
        media_files::Entity::find().count(&target.db).await?,
        count as u64
    );
    eprintln!(
        "phase=initial elapsed_ms={} requests={} body_bytes={} page_operations={} peak_rss_kib={:?}",
        initial.elapsed().as_millis(),
        traffic.requests.load(Ordering::Relaxed),
        traffic.bytes.load(Ordering::Relaxed),
        traffic.pages.load(Ordering::Relaxed),
        peak_rss_kib()
    );
    eprintln!(
        "phase=initial operations={:?}",
        traffic.operations.lock().unwrap()
    );
    // Both directions must observe the state changes created by first contact.
    let mut stable = false;
    for _ in 0..8 {
        let before = (source.peer.head().await?, target.peer.head().await?);
        synchronize().await?;
        if (source.peer.head().await?, target.peer.head().await?) == before {
            stable = true;
            break;
        }
    }
    assert!(stable, "initial exchange did not settle");
    traffic.reset();
    let incremental = Instant::now();
    let key = database::sync::model::ObjectKey {
        collection: database::sync::model::Collection::Tracks,
        id: Uuid::from_u128(1),
    };
    let database::sync::model::Payload::Track(mut value) =
        source.library.get(&key).await?.unwrap().value.unwrap()
    else {
        unreachable!()
    };
    value.metadata[0].value = "Edited track".into();
    source
        .library
        .write_local(
            key.clone(),
            Some(database::sync::model::Payload::Track(value.clone())),
        )
        .await?;
    synchronize().await?;
    assert_eq!(
        target.library.get(&key).await?.unwrap().value,
        Some(database::sync::model::Payload::Track(value))
    );
    eprintln!(
        "phase=incremental edits=1 elapsed_ms={} requests={} body_bytes={} page_operations={} peak_rss_kib={:?}",
        incremental.elapsed().as_millis(),
        traffic.requests.load(Ordering::Relaxed),
        traffic.bytes.load(Ordering::Relaxed),
        traffic.pages.load(Ordering::Relaxed),
        peak_rss_kib()
    );
    synchronize().await?;
    traffic.reset();
    let before = (source.peer.head().await?, target.peer.head().await?);
    let idle = Instant::now();
    for _ in 0..3 {
        synchronize().await?;
    }
    eprintln!(
        "phase=idle rounds=3 elapsed_ms={} requests={} body_bytes={} page_operations={} blob_operations={}",
        idle.elapsed().as_millis(),
        traffic.requests.load(Ordering::Relaxed),
        traffic.bytes.load(Ordering::Relaxed),
        traffic.pages.load(Ordering::Relaxed),
        traffic.blobs.load(Ordering::Relaxed)
    );
    assert_eq!(
        (source.peer.head().await?, target.peer.head().await?),
        before
    );
    assert_eq!(traffic.pages.load(Ordering::Relaxed), 0);
    assert_eq!(traffic.blobs.load(Ordering::Relaxed), 0);
    server.abort();
    Ok(())
}

fn peak_rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        line.strip_prefix("VmHWM:")
            .and_then(|value| value.split_whitespace().next()?.parse().ok())
    })
}
