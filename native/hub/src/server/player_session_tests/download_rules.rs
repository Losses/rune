use super::fixture;
use crate::{
    Signal,
    messages::{MixQuery, RemoveDownloadRuleRequest, SetDownloadRuleRequest},
    server::sync::downloads::{DownloadAudioContext, DownloadProgress, download_audio_pass},
    test_wait::TEST_DEADLINE,
    utils::ParamsExtractor,
};
use anyhow::{Context, Result};
use axum::{body::Body, extract::Path, http::StatusCode, routing::get};
use database::{
    actions::file::{hash_file, is_materialized},
    entities::media_files,
    sync::model::{Collection, ObjectKey, RuneModel},
    test_fixtures::causal_library as causal,
};
use futures::{StreamExt, stream};
use http_request::sync::{MetadataPeer, router};
use reqwest::Client;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::{
    collections::HashMap,
    fs, io,
    sync::{Arc, Mutex},
};
use tokio::{
    net::TcpListener,
    sync::Notify,
    time::{Instant, timeout},
};
use uuid::Uuid;

#[tokio::test]
async fn disabling_an_offline_rule_stops_before_the_next_file() -> Result<()> {
    exercise_rule_change(RuleEdit::Disable, Cleanup::Off).await
}

#[tokio::test]
async fn removing_an_offline_rule_stops_before_the_next_file() -> Result<()> {
    exercise_rule_change(RuleEdit::Remove, Cleanup::Off).await
}

#[tokio::test]
async fn cleanup_does_not_attach_the_unselected_file_in_flight() -> Result<()> {
    exercise_rule_change(RuleEdit::Remove, Cleanup::Unselected).await
}

#[tokio::test]
async fn overlapping_rules_keep_the_in_flight_file_and_download_the_next() -> Result<()> {
    exercise_rule_change(RuleEdit::Disable, Cleanup::WithOverlap).await
}

enum RuleEdit {
    Disable,
    Remove,
}

enum Cleanup {
    Off,
    Unselected,
    WithOverlap,
}

async fn exercise_rule_change(edit: RuleEdit, cleanup: Cleanup) -> Result<()> {
    let overlap = matches!(cleanup, Cleanup::WithOverlap);
    let cleanup = !matches!(cleanup, Cleanup::Off);
    let dir = tempfile::tempdir()?;
    let node = fixture(dir.path()).await?;
    node.stop_worker().await?;
    let params = &node.global_params;
    let library = params.test_library();
    let mut blobs = HashMap::new();
    let mut files = HashMap::new();
    for index in 0..2 {
        let path = causal::fixtures::audio_fixture_at(index);
        let blob = hash_file(&library.fsio, &path)?;
        blobs.insert(blob.digest.clone(), fs::read(path)?);
        let track = Uuid::new_v4();
        library
            .library_sync
            .write_local(
                ObjectKey {
                    collection: Collection::Tracks,
                    id: track,
                },
                Some(causal::audio(format!("Offline track {index}"), &blob)),
            )
            .await?;
        let file = media_files::Entity::find()
            .filter(media_files::Column::SourceUuid.eq(track.to_string()))
            .one(&*library.main_db)
            .await?
            .context("projected track")?;
        files.insert(blob.digest, file.id);
    }
    let request = SetDownloadRuleRequest {
        request_id: "enable".into(),
        queries: vec![MixQuery {
            operator: "lib::all".into(),
            parameter: "true".into(),
        }],
        mix_id: None,
        enabled: true,
        delete_unselected_files: false,
        call_id: None,
    };
    let rule = request
        .handle(request.extract_params(params), None, &request)
        .await?
        .context("enable response")?
        .rule_id;

    let blobs = Arc::new(blobs);
    let started = Arc::new(Notify::new());
    let finish = Arc::new(Notify::new());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let listen = TcpListener::bind("127.0.0.1:0").await?;
    let address = listen.local_addr()?;
    let service = router::<RuneModel>(Arc::new(library.library_sync.endpoint())).route(
        "/sync/blobs/{digest}",
        get({
            let started = started.clone();
            let finish = finish.clone();
            let requests = requests.clone();
            move |Path(digest): Path<String>| {
                let body = blobs.get(&digest).cloned();
                let started = started.clone();
                let finish = finish.clone();
                let requests = requests.clone();
                async move {
                    let Some(body) = body else {
                        return (StatusCode::NOT_FOUND, Body::empty());
                    };
                    let first = {
                        let mut requests = requests.lock().unwrap();
                        let first = requests.is_empty();
                        requests.push(digest);
                        first
                    };
                    if !first {
                        return (StatusCode::OK, Body::from(body));
                    }
                    let split = body.len() / 2;
                    let first = body[..split].to_vec();
                    let second = body[split..].to_vec();
                    let stream =
                        stream::iter([Ok::<_, io::Error>(first)]).chain(stream::once(async move {
                            started.notify_one();
                            finish.notified().await;
                            Ok::<_, io::Error>(second)
                        }));
                    (StatusCode::OK, Body::from_stream(stream))
                }
            }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listen, service).await });
    let peer = MetadataPeer::new(format!("http://{address}"), Client::new());
    let node_id = Uuid::parse_str(&params.node.node_id)?;
    let context = || DownloadAudioContext {
        rules: &library.download_rules,
        node_id,
        remote: &peer,
        notify: Arc::new(|| {}),
        playback_queue: &[],
    };
    let mut progress = DownloadProgress::default();
    let transfer = download_audio_pass(
        library.main_db.as_ref().clone(),
        &library.recommend_db,
        library.fsio.clone(),
        library.media_root.clone().into(),
        context(),
        Instant::now() + TEST_DEADLINE,
        &mut progress,
    );
    let change = async {
        timeout(TEST_DEADLINE, started.notified()).await?;
        if overlap {
            // These individual rules exist at the time the broad rule is withdrawn.
            for file in files.values() {
                let request = SetDownloadRuleRequest {
                    request_id: format!("overlap-{file}"),
                    queries: vec![MixQuery {
                        operator: "lib::track".into(),
                        parameter: file.to_string(),
                    }],
                    mix_id: None,
                    enabled: true,
                    delete_unselected_files: false,
                    call_id: None,
                };
                request
                    .handle(request.extract_params(params), None, &request)
                    .await?
                    .context("overlapping rule response")?;
            }
        }
        if matches!(edit, RuleEdit::Remove) {
            let request = RemoveDownloadRuleRequest {
                request_id: "remove".into(),
                rule_id: rule,
                delete_unselected_files: cleanup,
                call_id: None,
            };
            request
                .handle(request.extract_params(params), None, &request)
                .await?
                .context("remove response")?;
        } else {
            let request = SetDownloadRuleRequest {
                request_id: "disable".into(),
                enabled: false,
                delete_unselected_files: cleanup,
                ..request
            };
            request
                .handle(request.extract_params(params), None, &request)
                .await?
                .context("disable response")?;
        }
        finish.notify_one();
        Ok::<_, anyhow::Error>(())
    };
    let (transfer, ()) =
        timeout(TEST_DEADLINE, async { tokio::try_join!(transfer, change) }).await??;
    let first = {
        let requests = requests.lock().unwrap();
        assert_eq!(
            requests.len(),
            1,
            "the stale rule started a second transfer"
        );
        files[&requests[0]]
    };
    let keep_first = !cleanup || overlap;
    assert_eq!(transfer.report.downloaded, usize::from(keep_first));
    assert_eq!(
        is_materialized(
            &*library.main_db,
            &library.fsio,
            std::path::Path::new(&library.media_root),
            first,
        )
        .await?,
        keep_first,
        "the in-flight file has the wrong final offline state"
    );
    if overlap {
        let next = download_audio_pass(
            library.main_db.as_ref().clone(),
            &library.recommend_db,
            library.fsio.clone(),
            library.media_root.clone().into(),
            context(),
            Instant::now() + TEST_DEADLINE,
            &mut progress,
        )
        .await?;
        assert_eq!(next.report.downloaded, 1);
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_ne!(
            requests[0], requests[1],
            "the next pass must fetch the other file"
        );
    }
    node.global_params.node.shutdown().await?;
    server.abort();
    Ok(())
}
