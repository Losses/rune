use super::*;
use crate::messages::{NextRequest, PauseRequest, PlayRequest};
use crate::service::registry::ServiceSetup;

const MPRIS_CHILD: &str = "RUNE_MPRIS_JUKEBOX_CASE";
const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";
const PLAYER_PATH: &str = "/org/mpris/MediaPlayer2";

#[test]
fn mpris_follows_the_selected_jukebox_and_restores_local_playback() -> Result<()> {
    if let Ok(case) = env::var(MPRIS_CHILD) {
        return Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()?
            .block_on(check_mpris_selection(case == "local-paused"));
    }
    let (_, module) = module_path!().split_once("::").context("test module")?;
    let test = format!("{module}::mpris_follows_the_selected_jukebox_and_restores_local_playback");
    // A private session bus also works in shells without an installed session.conf.
    let dbus_config = tempfile::NamedTempFile::new()?;
    fs::write(
        dbus_config.path(),
        r#"<busconfig>
            <type>session</type>
            <listen>unix:tmpdir=/tmp</listen>
            <policy context="default">
                <allow send_destination="*"/>
                <allow receive_sender="*"/>
                <allow own="*"/>
            </policy>
        </busconfig>"#,
    )?;
    let mut failures = Vec::new();
    for case in ["empty-local", "local-paused"] {
        let status = Command::new("timeout")
            .args(["90", "dbus-run-session", "--config-file"])
            .arg(dbus_config.path())
            .arg("--")
            .arg(env::current_exe()?)
            .args(["--exact", &test, "--nocapture"])
            .env(MPRIS_CHILD, case)
            .status()?;
        if !status.success() {
            failures.push((case, status));
        }
    }
    anyhow::ensure!(failures.is_empty(), "MPRIS child test failed: {failures:?}");
    Ok(())
}

async fn check_mpris_selection(local_paused: bool) -> Result<()> {
    let dir = tempfile::tempdir()?;
    let signals = Arc::new(RecordedSignals::default());
    let library_id = Uuid::new_v4();
    let fixture = super::super::fixture_with_setup(
        dir.path(),
        library_id,
        Duration::from_secs(300),
        signals.clone(),
        ReconnectConfig::default(),
        (
            RunMode::App,
            crate::session::PlayerSession::real_player(),
            true,
        ),
        ServiceSetup::default(),
    )
    .await?;
    let node = fixture.global_params.node.clone();
    let registry = LibraryRegistry::load(dir.path())?;
    let record = registry
        .libraries
        .iter()
        .find(|record| record.library_id == Some(library_id))
        .context("local library record")?;
    let bus_name = format!(
        "org.mpris.MediaPlayer2.rune_player_{}_local",
        node.node_id.replace('-', "_")
    );
    let mut local_title = None;
    if local_paused {
        let file = session_audio(&fixture, "local-mpris-track.ogg").await?;
        dispatch_gui_request(
            &fixture.global_params,
            &LoadResolvedItemsRequest {
        controlling_device: None,
                call_id: None,
                items: vec![PlayingItemRef::from(
                    library_track_for_id(&fixture, file).await?,
                )],
                start_index: Some(0),
                mode: PlaylistOperateMode::Replace,
                expected_revision: None,
                playback_mode: None,
                position_seconds: None,
                play: LoadPlay::Play,
            },
        )
        .await?;
        wait_player(&fixture, |status| {
            status.state == PlayerPlaybackState::Playing
        })
        .await?;
        dispatch_gui_request(&fixture.global_params, &PauseRequest { call_id: None }).await?;
        local_title = Some(
            wait_until("the paused local report", || async {
                Ok(signals
                    .decoded::<PlaybackStatus>("PlaybackStatus")
                    .into_iter()
                    .filter(|status| status.ready && status.state == PlaybackState::Paused)
                    .filter_map(|status| status.title)
                    .find(|title| !title.is_empty()))
            })
            .await?,
        );
    }
    let local_state = if local_paused { "Paused" } else { "Stopped" };
    wait_for_mpris(&bus_name, local_state, local_title.as_deref()).await?;

    let (endpoint, server) = jukebox_server().await?;
    open_subsonic_jukebox(node.clone(), endpoint).await?;
    let remote = node.current_ui().await?;
    dispatch_gui_request(&remote, &PlayRequest { call_id: None }).await?;
    expect_remote(&signals, &bus_name, "Jukebox A", PlaybackState::Playing).await?;
    dispatch_gui_request(&remote, &PauseRequest { call_id: None }).await?;
    expect_remote(&signals, &bus_name, "Jukebox A", PlaybackState::Paused).await?;
    dispatch_gui_request(&remote, &PlayRequest { call_id: None }).await?;
    dispatch_gui_request(&remote, &NextRequest { call_id: None }).await?;
    expect_remote(&signals, &bus_name, "Jukebox B", PlaybackState::Playing).await?;
    if local_paused {
        assert_eq!(
            fixture.global_params.player.lock().await.get_status().state,
            PlayerPlaybackState::Paused
        );
    }

    super::super::open_registered_library_record(node.clone(), record.record_id.0.to_string())
        .await?;
    wait_for_mpris(&bus_name, local_state, local_title.as_deref()).await?;
    server.abort();
    node.shutdown().await
}

async fn expect_remote(
    signals: &RecordedSignals,
    bus_name: &str,
    title: &str,
    state: PlaybackState,
) -> Result<()> {
    wait_until("the selected jukebox report", || async {
        Ok(signals
            .decoded::<PlaybackStatus>("PlaybackStatus")
            .into_iter()
            .any(|status| status.title.as_deref() == Some(title) && status.state == state)
            .then_some(()))
    })
    .await?;
    let state = match state {
        PlaybackState::Playing => "Playing",
        PlaybackState::Paused => "Paused",
        PlaybackState::Stopped => "Stopped",
    };
    wait_for_mpris(bus_name, state, Some(title)).await
}

#[derive(Default)]
struct Jukebox {
    index: usize,
    playing: bool,
}

async fn jukebox_server() -> Result<(String, JoinHandle<()>)> {
    let listener = TokioTcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let app = Router::new()
        .route("/rest/{*endpoint}", get(jukebox_response))
        .with_state(Arc::new(StdMutex::new(Jukebox::default())));
    let server = task::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve test jukebox");
    });
    Ok((format!("http://{address}"), server))
}

async fn jukebox_response(
    State(jukebox): State<Arc<StdMutex<Jukebox>>>,
    path: AxumPath<String>,
    query: Query<HashMap<String, String>>,
) -> axum::response::Response {
    let signed_in = query.get("u").is_some_and(|user| user == "alice")
        && query
            .get("s")
            .zip(query.get("t"))
            .is_some_and(|(salt, token)| {
                let expected = md5::compute(format!("player-change-test-password{salt}"));
                *token == format!("{expected:x}")
            });
    if !signed_in {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    if path.0.trim_end_matches(".view") == "jukeboxControl" {
        let mut jukebox = jukebox.lock().unwrap();
        match query.get("action").map(String::as_str) {
            Some("start") => jukebox.playing = true,
            Some("stop") => jukebox.playing = false,
            Some("skip") => jukebox.index = query["index"].parse().unwrap(),
            _ => (),
        }
        let status = json!({
            "currentIndex": jukebox.index,
            "playing": jukebox.playing,
            "gain": 1.0,
            "position": 7
        });
        let mut playlist = status.clone();
        playlist["entry"] = json!([
            {"id":"track-a","title":"Jukebox A","duration":180},
            {"id":"track-b","title":"Jukebox B","duration":180}
        ]);
        return axum::Json(json!({
            "subsonic-response": {
                "status":"ok",
                "jukeboxStatus": status,
                "jukeboxPlaylist": playlist
            }
        }))
        .into_response();
    }
    fake_subsonic_response(
        State(FakeSubsonicRoles {
            stream: true,
            jukebox: true,
            wrong_password: false,
        }),
        path,
        query,
    )
    .await
    .into_response()
}

fn mpris_property(bus_name: &str, property: &str) -> Result<serde_json::Value> {
    let reply = Command::new("busctl")
        .args([
            "--user",
            "--json=short",
            "--timeout=2s",
            "get-property",
            bus_name,
            PLAYER_PATH,
            PLAYER_INTERFACE,
            property,
        ])
        .output()?;
    anyhow::ensure!(
        reply.status.success(),
        "busctl {property}: {}",
        String::from_utf8_lossy(&reply.stderr)
    );
    Ok(serde_json::from_slice(&reply.stdout)?)
}

fn mpris_snapshot(bus_name: &str) -> Result<(String, Option<String>)> {
    let status = mpris_property(bus_name, "PlaybackStatus")?;
    let metadata = mpris_property(bus_name, "Metadata")?;
    Ok((
        status["data"]
            .as_str()
            .context("MPRIS PlaybackStatus")?
            .into(),
        metadata["data"]["xesam:title"]["data"]
            .as_str()
            .map(str::to_owned),
    ))
}

async fn wait_for_mpris(bus_name: &str, state: &str, title: Option<&str>) -> Result<()> {
    let mut poll = tokio_time::interval(Duration::from_millis(100));
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        poll.tick().await;
        let observed = mpris_snapshot(bus_name);
        if observed.as_ref().is_ok_and(|(actual_state, actual_title)| {
            actual_state == state && actual_title.as_deref() == title
        }) {
            return Ok(());
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "MPRIS expected {state} with title {title:?}, got {observed:?}"
        );
    }
}
