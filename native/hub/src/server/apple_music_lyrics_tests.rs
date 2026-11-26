// The lyrics of an Apple Music library's songs, looked up at a fake LRCLIB once the user allows
// it for the library.

use std::collections::HashMap;

use crate::messages::{LyricContentLine, SetOnlineLyricsRequest};

const SYNCED_LYRICS: &str = r#"{"duration":200.0,"instrumental":false,"plainLyrics":"one\ntwo","syncedLyrics":"[00:01.00]one\n[00:03.50]two"}"#;

/// Allows or stops the lookups of lyrics online for the library `record`, as the settings row
/// does.
async fn set_online_lyrics(
    node: &Arc<NodeContext>,
    record: LibraryRecordId,
    enabled: bool,
) -> Result<()> {
    let request = SetOnlineLyricsRequest {
        record_id: record.0.to_string(),
        enabled,
        call_id: None,
    };
    request.handle(node.clone(), None, &request).await?;
    Ok(())
}

/// An Apple Music library whose user allowed looking lyrics up online, and the fake LRCLIB it
/// looks them up at.
async fn open_apple_with_lyrics(dir: &Path) -> Result<(OpenApple, Arc<FakeLrclib>)> {
    let apple = open_apple(dir).await?;
    set_online_lyrics(&apple.node, apple.record, true).await?;
    let lrclib = apple.lrclib.clone();
    Ok((apple, lrclib))
}

async fn lyric_lines(apple: &OpenApple, track: &str) -> Result<Vec<LyricContentLine>> {
    Ok(dispatch_gui_request(
        &apple.fixture.global_params,
        &GetLyricByTrackIdRequest {
            call_id: None,
            item: Some(song_ref(apple, track)),
        },
    )
    .await?
    .context("lyrics")?
    .lines)
}

/// What the record summaries say of each record's lookups of lyrics online, by record id.
async fn listed_online_lyrics(node: &Arc<NodeContext>) -> Result<HashMap<String, Option<bool>>> {
    use crate::messages::ListLibraryRecordsRequest;

    let request = ListLibraryRecordsRequest { call_id: None };
    let listed = request
        .handle(node.clone(), None, &request)
        .await?
        .context("list library records")?;
    Ok(listed
        .records
        .into_iter()
        .map(|summary| (summary.record_id, summary.online_lyrics))
        .collect())
}

/// What the binding of `apple` declares for the lyrics of its items.
fn lyric_capability(apple: &OpenApple) -> RequestCapability {
    use crate::macros::item_capability_snapshot;
    use crate::request_owner::Implementation;

    item_capability_snapshot(
        None,
        &Implementation::Service(apple.binding.clone()).item_slots(),
    )
    .get_lyric_by_track_id_request
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_apple_song_gets_its_synced_lyrics_from_lrclib_and_keeps_them() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (apple, lrclib) = open_apple_with_lyrics(dir.path()).await?;
    lrclib.answer("/api/get", Some((200, SYNCED_LYRICS)));
    let lines = lyric_lines(&apple, "i.s1").await?;
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].sections[0].content, "one");
    assert_eq!(lines[1].start_time, 3500);
    let asked = lrclib.asked();
    assert_eq!(asked.len(), 1, "{asked:?}");
    // The song is asked for by its title, credit, album and length.
    assert!(
        asked[0].contains("track_name=First+Light&artist_name=Artist+One&album_name=Dawn+Album"),
        "{asked:?}"
    );
    assert!(asked[0].ends_with("&duration=200"), "{asked:?}");
    // A replay does not ask again, even when LRCLIB is gone.
    lrclib.answer("/api/get", None);
    assert_eq!(lyric_lines(&apple, "i.s1").await?.len(), 2);
    assert_eq!(lrclib.asked().len(), 1);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_an_exact_match_the_search_picks_the_result_of_the_songs_length() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (apple, lrclib) = open_apple_with_lyrics(dir.path()).await?;
    lrclib.answer("/api/get", Some((404, "{}")));
    lrclib.answer(
        "/api/search",
        Some((
            200,
            r#"[
                {"trackName":"First Light","duration":260,"instrumental":false,"plainLyrics":"wrong","syncedLyrics":null},
                {"trackName":"first light","duration":201,"instrumental":false,"plainLyrics":"right","syncedLyrics":null}
            ]"#,
        )),
    );
    let lines = lyric_lines(&apple, "i.s1").await?;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].sections[0].content, "right");
    assert_eq!(lrclib.asked().len(), 2);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn plain_lyrics_show_untimed_and_an_instrumental_has_no_lines_and_is_not_asked_again()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let (apple, lrclib) = open_apple_with_lyrics(dir.path()).await?;
    lrclib.answer(
        "/api/get",
        Some((
            200,
            r#"{"duration":200,"instrumental":false,"plainLyrics":"one\ntwo","syncedLyrics":null}"#,
        )),
    );
    let lines = lyric_lines(&apple, "i.s1").await?;
    assert_eq!(lines.len(), 2);
    assert_eq!((lines[0].start_time, lines[0].end_time), (0, 0));
    lrclib.answer(
        "/api/get",
        Some((
            200,
            r#"{"duration":200,"instrumental":true,"plainLyrics":null,"syncedLyrics":null}"#,
        )),
    );
    assert!(lyric_lines(&apple, "i.s2").await?.is_empty());
    let asked = lrclib.asked().len();
    assert!(lyric_lines(&apple, "i.s2").await?.is_empty());
    assert_eq!(lrclib.asked().len(), asked);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_miss_is_kept_but_a_failure_to_ask_is_not() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (apple, lrclib) = open_apple_with_lyrics(dir.path()).await?;
    lrclib.answer("/api/get", Some((404, "{}")));
    lrclib.answer("/api/search", Some((200, "[]")));
    assert!(lyric_lines(&apple, "i.s1").await?.is_empty());
    assert_eq!(lrclib.asked().len(), 2);
    assert!(lyric_lines(&apple, "i.s1").await?.is_empty());
    assert_eq!(lrclib.asked().len(), 2, "a miss was asked for again");

    // Offline: no lines and no error, and the song is asked about again once back online.
    lrclib.answer("/api/get", None);
    assert!(lyric_lines(&apple, "i.s2").await?.is_empty());
    assert!(lyric_lines(&apple, "i.s2").await?.is_empty());
    assert_eq!(lrclib.asked().len(), 4);
    lrclib.answer("/api/get", Some((200, SYNCED_LYRICS)));
    assert_eq!(lyric_lines(&apple, "i.s2").await?.len(), 2);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_rate_limit_is_not_a_miss_and_holds_back_every_lookup_for_a_while() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (apple, lrclib) = open_apple_with_lyrics(dir.path()).await?;
    lrclib.answer("/api/get", Some((429, "")));
    assert!(lyric_lines(&apple, "i.s1").await?.is_empty());
    assert_eq!(lrclib.asked().len(), 1, "a lookup was retried");
    // Another song is not asked about while LRCLIB asked to be left alone.
    lrclib.answer("/api/get", Some((200, SYNCED_LYRICS)));
    assert!(lyric_lines(&apple, "i.s2").await?.is_empty());
    assert_eq!(lrclib.asked().len(), 1);
    // Once the wait is over, the songs are asked about again: the refusal was no answer.
    apple.clock.advance(std::time::Duration::from_secs(61));
    assert_eq!(lyric_lines(&apple, "i.s1").await?.len(), 2);
    assert_eq!(lrclib.asked().len(), 2);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_apple_song_has_no_lyrics_and_asks_no_one_until_the_user_allows_the_lookups()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let heard = Arc::new(Heard::default());
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            heard: Some(heard.clone()),
            ..AppleOptions::default()
        },
    )
    .await?;
    let node = &apple.node;
    let lrclib = apple.lrclib.clone();
    lrclib.answer("/api/get", Some((200, SYNCED_LYRICS)));
    let announced = || {
        heard
            .placed::<crate::messages::LibraryRecordsChanged>("LibraryRecordsChanged")
            .into_iter()
            .filter_map(|(_, changed)| {
                changed
                    .records
                    .into_iter()
                    .find(|summary| summary.record_id == apple.record.0.to_string())
                    .map(|summary| summary.online_lyrics)
            })
            .next_back()
    };

    // A new library is signed in, and still its songs have no lyrics: nothing goes to LRCLIB.
    assert!(apple.binding.is_signed_in());
    assert_eq!(lyric_capability(&apple), RequestCapability::Hidden);
    let refused = lyric_lines(&apple, "i.s1")
        .await
        .err()
        .context("lyrics before the lookups were allowed")?;
    assert!(unsupported(&refused), "{refused:#}");
    assert!(lrclib.asked().is_empty());

    // The user allows the lookups: the songs have lyrics, which are looked up.
    set_online_lyrics(node, apple.record, true).await?;
    assert_eq!(lyric_capability(&apple), RequestCapability::Available);
    assert_eq!(announced(), Some(Some(true)));
    assert_eq!(lyric_lines(&apple, "i.s1").await?.len(), 2);
    assert_eq!(lrclib.asked().len(), 1);

    // The user stops them: the songs have no lyrics again, not even the ones kept, and another
    // song is not asked about.
    set_online_lyrics(node, apple.record, false).await?;
    assert_eq!(lyric_capability(&apple), RequestCapability::Hidden);
    assert_eq!(announced(), Some(Some(false)));
    for track in ["i.s1", "i.s2"] {
        let refused = lyric_lines(&apple, track)
            .await
            .err()
            .context("lyrics after the lookups were stopped")?;
        assert!(unsupported(&refused), "{refused:#}");
    }
    assert_eq!(lrclib.asked().len(), 1);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_ui_hears_whether_the_playing_apple_song_has_lyrics_when_the_user_changes_the_lookups()
-> Result<()> {
    use crate::messages::{LoadPlay, PlayingItemCapabilities};

    let dir = tempfile::tempdir()?;
    let (apple, heard) = open_apple_heard(dir.path()).await?;
    let song = song_ref(&apple, "i.s1");
    apple
        .run(&load_items(vec![song.clone()], LoadPlay::Pause, 30.0))
        .await?;
    let told = told_before_its_status(&heard, &song).await?;
    assert_eq!(
        told.get_lyric_by_track_id_request,
        RequestCapability::Hidden,
        "{told:?}"
    );
    // The rest is as for any signed-in Apple Music library.
    assert_eq!(told.get_liked_request, RequestCapability::Available);

    for (enabled, capability) in [
        (true, RequestCapability::Available),
        (false, RequestCapability::Hidden),
    ] {
        set_online_lyrics(&apple.node, apple.record, enabled).await?;
        wait_until("the UI to hear whether the song has lyrics", || async {
            Ok(heard
                .placed::<PlayingItemCapabilities>("PlayingItemCapabilities")
                .into_iter()
                .map(|(_, told)| told)
                .rfind(|told| told.item.as_ref() == Some(&song))
                .filter(|told| told.get_lyric_by_track_id_request == capability)
                .map(|_| ()))
        })
        .await?;
    }
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn allowed_lookups_of_lyrics_survive_a_restart_and_a_sign_out() -> Result<()> {
    use crate::macros::item_capability_snapshot;
    use crate::messages::SignOutServiceLibraryRequest;
    use crate::request_owner::Implementation;
    use crate::service::unbound_item_slots;
    use crate::utils::sign_out_service_library_on_node;

    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let record = apple.record;
    let id = record.0.to_string();
    set_online_lyrics(&apple.node, record, true).await?;

    // The app quits and starts again, and holds no binding for the library yet: its record and
    // what this device keeps say the choice.
    let restarted = restart_apple(apple).await?;
    let node = &restarted.node;
    assert!(node.services.apple_music_binding(record).is_none());
    assert_eq!(
        listed_online_lyrics(node).await?.get(&id),
        Some(&Some(true))
    );
    let unbound = |node: &NodeContext| {
        let slots = unbound_item_slots(
            Path::new(&*node.config_path),
            ServiceKind::AppleMusic,
            record.0,
        );
        item_capability_snapshot(None, &slots).get_lyric_by_track_id_request
    };
    assert_eq!(unbound(node), RequestCapability::Available);

    // The binding the next sign-in makes looks lyrics up without being told again.
    restarted.sign_in(USER_TOKEN).await?;
    let binding = node
        .services
        .apple_music_binding(record)
        .context("signed-in binding")?;
    assert!(binding.looks_up_lyrics());

    // Signing out keeps the choice.
    let sign_out = SignOutServiceLibraryRequest {
        record_id: id.clone(),
        call_id: None,
    };
    sign_out_service_library_on_node(&sign_out, node).await?;
    assert!(binding.looks_up_lyrics());
    let held = || {
        item_capability_snapshot(None, &Implementation::Service(binding.clone()).item_slots())
            .get_lyric_by_track_id_request
    };
    assert_eq!(held(), RequestCapability::Available);
    assert_eq!(
        listed_online_lyrics(node).await?.get(&id),
        Some(&Some(true))
    );

    // Stopping the lookups is kept too, and the binding held follows it.
    set_online_lyrics(node, record, false).await?;
    assert!(!binding.looks_up_lyrics());
    assert_eq!(held(), RequestCapability::Hidden);
    assert_eq!(unbound(node), RequestCapability::Hidden);
    assert_eq!(
        listed_online_lyrics(node).await?.get(&id),
        Some(&Some(false))
    );
    restarted.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_a_library_whose_lyrics_come_from_online_says_whether_it_looks_them_up() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let subsonic = write_subsonic_record(&apple.node, "http://127.0.0.1:9".into()).await?;
    let apple_id = apple.record.0.to_string();

    let listed = listed_online_lyrics(&apple.node).await?;
    assert_eq!(listed.get(&apple_id), Some(&Some(false)));
    // The fixture's local library and a Subsonic library offer no such lookups.
    assert!(listed.len() >= 3, "{listed:?}");
    for (record, online_lyrics) in &listed {
        if *record != apple_id {
            assert_eq!(*online_lyrics, None, "{record}");
        }
    }
    // Nor can they be turned on for one of them, or for a record that does not exist.
    for record in [subsonic, LibraryRecordId(uuid::Uuid::new_v4())] {
        let refused = set_online_lyrics(&apple.node, record, true)
            .await
            .err()
            .context("lookups for a library without online lyrics")?;
        assert!(invalid_input(&refused), "{refused:#}");
    }
    assert_eq!(
        listed_online_lyrics(&apple.node)
            .await?
            .get(&subsonic.0.to_string()),
        Some(&None)
    );

    set_online_lyrics(&apple.node, apple.record, true).await?;
    assert_eq!(
        listed_online_lyrics(&apple.node).await?.get(&apple_id),
        Some(&Some(true))
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_choice_and_the_kept_lyrics_go_with_the_record() -> Result<()> {
    use crate::service::online_lyrics::{allowed, kept_lyrics_folder};

    let dir = tempfile::tempdir()?;
    let (apple, lrclib) = open_apple_with_lyrics(dir.path()).await?;
    lrclib.answer("/api/get", Some((200, SYNCED_LYRICS)));
    assert_eq!(lyric_lines(&apple, "i.s1").await?.len(), 2);
    let config = Path::new(&*apple.node.config_path).to_owned();
    let folder = kept_lyrics_folder(&config, apple.record.0);
    wait_until("the lyrics to be kept on disk", || async {
        Ok(folder.join("lyrics.json").exists().then_some(()))
    })
    .await?;

    // Records the user made outlive the record, and these go with it all the same.
    remove_apple(&apple, apple.record, false).await?;
    assert!(!folder.exists());
    assert!(!allowed(&config, apple.record.0));
    apple.finish().await
}
