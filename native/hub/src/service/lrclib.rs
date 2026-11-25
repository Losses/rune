//! Lyrics from LRCLIB (https://lrclib.net), a free database of synced and plain lyrics that
//! needs no key. It looks a song up by its title, artist, album and length, and keeps what it
//! learned so a song is not asked for again.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use futures::future::BoxFuture;
use lyric::lrc::parse_lrc;
use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;
use uuid::Uuid;

use crate::messages::{LyricContentLine, LyricContentLineSection};
use crate::service::online_lyrics::{self, kept_lyrics_folder};
use crate::service::registry::Clock;
use crate::service::store::ListStore;

/// Where LRCLIB answers.
const BASE_URL: &str = "https://lrclib.net";

/// How long the database may take to answer before the lookup gives up.
const TIMEOUT: Duration = Duration::from_secs(5);

/// How far a result's length may be from the song's, in seconds.
const DURATION_TOLERANCE: f64 = 2.0;

/// How long a song LRCLIB had nothing for waits before it is asked about again.
const MISS_RETRY_SECONDS: u64 = 24 * 60 * 60;

/// How many answers are kept; the oldest go first.
const MAX_KEPT: usize = 5000;

/// What the database answered a request with.
pub(crate) struct HttpAnswer {
    pub(crate) status: u16,
    pub(crate) body: String,
    /// The seconds a `Retry-After` header asked to wait, when it gave a number.
    pub(crate) retry_after: Option<u64>,
}

/// The database asked to be left alone for a while.
#[derive(Debug)]
struct RateLimited(Duration);

impl fmt::Display for RateLimited {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the lyrics database asked to wait {:?}", self.0)
    }
}

impl std::error::Error for RateLimited {}

/// How long to wait when a refusal names no time, and the longest wait obeyed.
const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(60);
const MAX_RETRY_AFTER: Duration = Duration::from_secs(3600);

fn refusal(answer: &HttpAnswer) -> anyhow::Error {
    if answer.status == 429 {
        let wait = answer
            .retry_after
            .map_or(DEFAULT_RETRY_AFTER, Duration::from_secs)
            .min(MAX_RETRY_AFTER);
        RateLimited(wait).into()
    } else {
        anyhow::anyhow!("the lyrics database answered {}", answer.status)
    }
}

/// Sends the lookups. A transport that cannot reach the database fails; any status it did
/// reach it with is an answer.
pub(crate) trait LrclibHttp: Send + Sync {
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<HttpAnswer>>;
}

/// The lookups sent over the network.
struct ReqwestLrclib {
    http: reqwest::Client,
}

impl ReqwestLrclib {
    fn new() -> Self {
        let agent = format!(
            "Rune/{} (https://github.com/losses/rune)",
            env!("CARGO_PKG_VERSION")
        );
        Self {
            http: reqwest::Client::builder()
                .user_agent(agent)
                .timeout(TIMEOUT)
                .build()
                .unwrap_or_default(),
        }
    }
}

impl LrclibHttp for ReqwestLrclib {
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<HttpAnswer>> {
        Box::pin(async move {
            let response = self
                .http
                .get(url)
                .send()
                .await
                .map_err(|error| anyhow::anyhow!("{}", describe(&error)))?;
            let status = response.status().as_u16();
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.trim().parse().ok());
            let body = response
                .text()
                .await
                .map_err(|error| anyhow::anyhow!("{}", describe(&error)))?;
            Ok(HttpAnswer {
                status,
                body,
                retry_after,
            })
        })
    }
}

/// What went wrong with a request, without its address, which carries the query.
fn describe(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "no answer in time"
    } else if error.is_connect() {
        "no connection"
    } else {
        "a failed connection"
    }
}

/// Where lyrics are looked up: the database at `base_url`, asked through `http`. A node is made
/// with LRCLIB itself, reached over the network; a test makes one with a database of its own.
#[derive(Clone)]
pub(crate) struct Lrclib {
    http: Arc<dyn LrclibHttp>,
    base_url: String,
}

impl Lrclib {
    pub(crate) fn new(http: Arc<dyn LrclibHttp>, base_url: impl Into<String>) -> Self {
        Self {
            http,
            base_url: base_url.into(),
        }
    }
}

impl Default for Lrclib {
    /// LRCLIB itself.
    fn default() -> Self {
        Self::new(Arc::new(ReqwestLrclib::new()), BASE_URL)
    }
}

impl fmt::Debug for Lrclib {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lrclib")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

/// The song to find lyrics for.
#[derive(Clone, Debug)]
pub(crate) struct SongQuery {
    pub(crate) title: String,
    pub(crate) artist: String,
    pub(crate) album: Option<String>,
    pub(crate) duration: Option<Duration>,
}

/// The lyrics LRCLIB has for a song.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Lyrics {
    /// LRC text with a time before each line.
    Synced(String),
    /// Lines without times.
    Plain(String),
    /// The song has no words.
    Instrumental,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    #[serde(default)]
    track_name: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    instrumental: bool,
    #[serde(default)]
    plain_lyrics: Option<String>,
    #[serde(default)]
    synced_lyrics: Option<String>,
}

impl Record {
    /// The lyrics it holds, the synced ones first.
    fn lyrics(self) -> Option<Lyrics> {
        if self.instrumental {
            return Some(Lyrics::Instrumental);
        }
        let present = |text: Option<String>| text.filter(|text| !text.trim().is_empty());
        present(self.synced_lyrics)
            .map(Lyrics::Synced)
            .or_else(|| present(self.plain_lyrics).map(Lyrics::Plain))
    }

    /// Whether the record is for a song titled `title`, as titles are compared.
    fn titled(&self, title: &str) -> bool {
        self.track_name
            .as_deref()
            .is_some_and(|found| normalised(found) == normalised(title))
    }

    /// Whether the record is as long as a song of `wanted` length, as far as it is known. A
    /// record without a length is not, for a song whose length is known.
    fn near(&self, wanted: Option<Duration>) -> bool {
        match (wanted, self.duration) {
            (Some(wanted), Some(found)) => {
                (found - wanted.as_secs_f64()).abs() <= DURATION_TOLERANCE
            }
            (Some(_), None) => false,
            (None, _) => true,
        }
    }
}

/// A title as titles are compared: without surrounding spaces, in lower case.
fn normalised(title: &str) -> String {
    title.trim().to_lowercase()
}

fn encoded(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn get_url(base_url: &str, song: &SongQuery) -> String {
    let mut url = format!(
        "{base_url}/api/get?track_name={}&artist_name={}",
        encoded(&song.title),
        encoded(&song.artist)
    );
    if let Some(album) = song.album.as_deref().filter(|album| !album.is_empty()) {
        url.push_str(&format!("&album_name={}", encoded(album)));
    }
    if let Some(duration) = song.duration {
        url.push_str(&format!(
            "&duration={}",
            duration.as_secs_f64().round() as u64
        ));
    }
    url
}

fn search_url(base_url: &str, song: &SongQuery) -> String {
    format!(
        "{base_url}/api/search?track_name={}&artist_name={}",
        encoded(&song.title),
        encoded(&song.artist)
    )
}

/// Looks the song up: by its exact facts first, and when the database has no such song and
/// the song's length is known, in its search for the first result of the same title and
/// length that has lyrics. `Ok(None)` is an answer that the database has none, which a song
/// without a title or artist gets without asking; an error is a failure to ask, which says
/// nothing about the song.
async fn find(lrclib: &Lrclib, song: &SongQuery) -> Result<Option<Lyrics>> {
    if song.title.trim().is_empty() || song.artist.trim().is_empty() {
        return Ok(None);
    }
    let answer = lrclib.http.get(&get_url(&lrclib.base_url, song)).await?;
    match answer.status {
        200 => {
            let record: Record =
                serde_json::from_str(&answer.body).context("an answer that could not be read")?;
            return Ok(record.lyrics());
        }
        // The database does not know the song, or does not take the question.
        400 | 404 => {}
        _ => return Err(refusal(&answer)),
    }
    // Without a length, a result cannot be told from another song's of the same title.
    if song.duration.is_none() {
        return Ok(None);
    }
    let answer = lrclib.http.get(&search_url(&lrclib.base_url, song)).await?;
    match answer.status {
        200 => {}
        400 => return Ok(None),
        _ => return Err(refusal(&answer)),
    }
    let records: Vec<Record> =
        serde_json::from_str(&answer.body).context("an answer that could not be read")?;
    Ok(records
        .into_iter()
        .filter(|record| record.titled(&song.title) && record.near(song.duration))
        .find_map(Record::lyrics))
}

/// One kept answer: the lyrics, or none, and when it was learned in seconds since the epoch.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Kept {
    lyrics: Option<Lyrics>,
    at: u64,
}

/// What was learned about each song, by the id the owner names it with. Lyrics found are kept
/// for good; a song without lyrics is asked about again after a day.
#[derive(Default)]
struct LyricsCache {
    entries: Entries,
}

/// The kept answers, shared with the saves: a save writes what is kept when it is made, and
/// costs a lookup no copy of them.
#[derive(Clone, Default)]
struct Entries(Arc<Mutex<HashMap<String, Kept>>>);

impl Entries {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Kept>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Serialize for Entries {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.lock().serialize(serializer)
    }
}

/// What a cache knows about a song.
enum Known {
    Lyrics(Lyrics),
    /// The database had none, lately.
    Missing,
    /// Nothing, or a miss old enough to ask again.
    Unknown,
}

impl LyricsCache {
    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<String, Kept>> {
        self.entries.lock()
    }

    fn known(&self, id: &str, now: u64) -> Known {
        match self.entries().get(id) {
            Some(Kept {
                lyrics: Some(lyrics),
                ..
            }) => Known::Lyrics(lyrics.clone()),
            Some(Kept { lyrics: None, at }) if now.saturating_sub(*at) < MISS_RETRY_SECONDS => {
                Known::Missing
            }
            _ => Known::Unknown,
        }
    }

    /// Keeps an answer and returns the kept answers to be saved, as they are when the save
    /// writes them.
    fn keep(&self, id: &str, lyrics: Option<Lyrics>, now: u64) -> Entries {
        let mut entries = self.entries();
        entries.insert(id.to_owned(), Kept { lyrics, at: now });
        while entries.len() > MAX_KEPT {
            let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, kept)| kept.at)
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            entries.remove(&oldest);
        }
        drop(entries);
        self.entries.clone()
    }

    /// Adds what was kept on disk before this run's own answers.
    fn restore(&self, stored: HashMap<String, Kept>) {
        let mut entries = self.entries();
        for (id, kept) in stored {
            entries.entry(id).or_insert(kept);
        }
    }
}

/// The file the kept answers are in, in the folder of the kept lyrics.
const LYRICS_FILE: &str = "lyrics";

/// The lyrics of the songs of one service library, looked up at LRCLIB once the user allowed it
/// for the library on this device, and kept there. Lyrics belong to no account, so a sign-out
/// keeps them; they go with the library's record.
pub(crate) struct LibraryLyrics {
    lrclib: Lrclib,
    /// What LRCLIB answered about each song.
    cache: LyricsCache,
    /// Where the answers are kept across restarts.
    store: ListStore,
    /// Whether the answers kept on disk were read.
    restored: OnceCell<()>,
    /// LRCLIB is not asked again before this, after it asked to be left alone.
    not_before: Mutex<Option<Instant>>,
    /// Whether the user allowed the lookups. It is kept on disk too, so it outlasts the binding
    /// that holds this.
    allowed: AtomicBool,
    /// The clock a wait LRCLIB asked for goes by.
    clock: Arc<dyn Clock>,
}

impl LibraryLyrics {
    /// The lyrics of the library `library_id`, kept in `config_dir` and looked up at `lrclib`
    /// as far as the user allowed it there. It reads that, so it blocks.
    pub(crate) fn new(
        config_dir: &Path,
        library_id: Uuid,
        lrclib: Lrclib,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            lrclib,
            cache: LyricsCache::default(),
            store: ListStore::new(kept_lyrics_folder(config_dir, library_id), None),
            restored: OnceCell::new(),
            not_before: Mutex::new(None),
            allowed: AtomicBool::new(online_lyrics::allowed(config_dir, library_id)),
            clock,
        }
    }

    /// Whether the user allowed looking the library's lyrics up.
    pub(crate) fn allowed(&self) -> bool {
        self.allowed.load(Ordering::SeqCst)
    }

    /// The user allowed or stopped the lookups, which is kept on disk already. The next lookup
    /// follows it.
    pub(crate) fn set_allowed(&self, allowed: bool) {
        self.allowed.store(allowed, Ordering::SeqCst);
    }

    /// The lyrics of `song`, kept under `id`: as kept, or else looked up at LRCLIB. A song it
    /// has none for is asked about again after a day. Failing to reach it is no answer: it is
    /// logged and nothing is kept, so the next request asks again, and a wait it asks for holds
    /// every lookup back. Without the user's leave there are none, and no one is asked.
    pub(crate) async fn lyrics_of(&self, id: &str, song: &SongQuery) -> Option<Lyrics> {
        if !self.allowed() {
            return None;
        }
        self.restored
            .get_or_init(|| async {
                let store = self.store.clone();
                let stored = tokio::task::spawn_blocking(move || {
                    store.load::<HashMap<String, Kept>>(LYRICS_FILE)
                })
                .await
                .ok()
                .flatten();
                if let Some(stored) = stored {
                    self.cache.restore(stored);
                }
            })
            .await;
        // The user may have stopped the lookups while the answers kept were read.
        if !self.allowed() {
            return None;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        match self.cache.known(id, now) {
            Known::Lyrics(lyrics) => return Some(lyrics),
            Known::Missing => return None,
            Known::Unknown => {}
        }
        let waiting = self
            .not_before
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some_and(|until| self.clock.now() < until);
        if waiting {
            return None;
        }
        let generation = self.store.generation();
        match find(&self.lrclib, song).await {
            Ok(found) => {
                let kept = self.cache.keep(id, found.clone(), now);
                self.store.save(generation, LYRICS_FILE, Arc::new(kept));
                found
            }
            Err(error) => {
                if let Some(RateLimited(wait)) = error.downcast_ref() {
                    *self
                        .not_before
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner) = Some(self.clock.now() + *wait);
                }
                log::warn!("Lyrics could not be looked up: {error}");
                None
            }
        }
    }
}

fn line(start: i32, end: i32, content: String) -> LyricContentLine {
    LyricContentLine {
        start_time: start,
        end_time: end,
        sections: vec![LyricContentLineSection {
            start_time: start,
            end_time: end,
            content,
        }],
    }
}

/// The lines the UI shows for lyrics: timed ones from LRC text; unsynced lyrics, which may
/// still carry times, as the Subsonic library shows them.
pub(crate) fn lines(lyrics: &Lyrics) -> Vec<LyricContentLine> {
    let (Lyrics::Synced(text) | Lyrics::Plain(text)) = lyrics else {
        return Vec::new();
    };
    if let Some(parsed) = parse_lrc(text)
        .ok()
        .map(|parsed| parsed.lyrics)
        .filter(|lines| !lines.is_empty())
    {
        return parsed
            .into_iter()
            .map(|parsed| {
                line(
                    parsed.start_time.into(),
                    parsed.end_time.into(),
                    parsed.text,
                )
            })
            .collect();
    }
    text.lines()
        .filter(|text| !text.trim().is_empty())
        .map(|text| line(0, 0, text.to_owned()))
        .collect()
}

#[cfg(test)]
pub(crate) mod fake {
    use std::sync::Mutex;

    use anyhow::bail;

    use super::*;

    type Scripted = Vec<(String, Option<(u16, String)>)>;

    /// Where the fake answers, which is not LRCLIB's own address.
    pub(crate) const FAKE_BASE_URL: &str = "http://lrclib.test";

    /// Answers by a path prefix, and counts what it was asked.
    #[derive(Default)]
    pub(crate) struct FakeLrclib {
        answers: Mutex<Scripted>,
        asked: Mutex<Vec<String>>,
    }

    impl FakeLrclib {
        /// Looking lyrics up at this fake.
        pub(crate) fn lrclib(self: &Arc<Self>) -> Lrclib {
            Lrclib::new(Arc::clone(self) as Arc<dyn LrclibHttp>, FAKE_BASE_URL)
        }

        /// Answers requests whose address starts with `prefix`; `None` fails to connect.
        pub(crate) fn answer(&self, prefix: &str, answer: Option<(u16, &str)>) {
            let mut answers = self.answers.lock().unwrap();
            answers.retain(|(kept, _)| kept != prefix);
            answers.push((
                prefix.to_owned(),
                answer.map(|(status, body)| (status, body.to_owned())),
            ));
        }

        pub(crate) fn asked(&self) -> Vec<String> {
            self.asked.lock().unwrap().clone()
        }
    }

    impl LrclibHttp for FakeLrclib {
        fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<HttpAnswer>> {
            self.asked.lock().unwrap().push(url.to_owned());
            // An address elsewhere reaches nothing.
            let answer = url.strip_prefix(FAKE_BASE_URL).and_then(|path| {
                self.answers
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|(prefix, _)| path.starts_with(prefix.as_str()))
                    .map(|(_, answer)| answer.clone())
            });
            Box::pin(async move {
                match answer {
                    Some(Some((status, body))) => Ok(HttpAnswer {
                        status,
                        body,
                        retry_after: None,
                    }),
                    Some(None) | None => bail!("no connection"),
                }
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeLrclib;
    use super::*;

    fn song(seconds: u64) -> SongQuery {
        SongQuery {
            title: "Night & Day".into(),
            artist: "Some Artist".into(),
            album: Some("The Album".into()),
            duration: Some(Duration::from_secs(seconds)),
        }
    }

    const SYNCED: &str = r#"{"duration":200.0,"instrumental":false,"plainLyrics":"one\ntwo","syncedLyrics":"[00:01.00]one\n[00:03.50]two"}"#;

    #[tokio::test]
    async fn a_song_is_asked_for_by_its_facts_with_the_duration_in_whole_seconds() {
        let http = Arc::new(FakeLrclib::default());
        http.answer("/api/get", Some((200, SYNCED)));
        let found = find(&http.lrclib(), &song(200)).await.unwrap();
        assert!(matches!(found, Some(Lyrics::Synced(_))));
        let asked = http.asked();
        assert_eq!(asked.len(), 1);
        // The database is asked where it was set up to answer.
        assert!(
            asked[0].starts_with("http://lrclib.test/api/get?"),
            "{asked:?}"
        );
        assert!(asked[0].contains("track_name=Night+%26+Day"), "{asked:?}");
        assert!(asked[0].contains("artist_name=Some+Artist"), "{asked:?}");
        assert!(asked[0].contains("album_name=The+Album"), "{asked:?}");
        assert!(asked[0].ends_with("&duration=200"), "{asked:?}");
    }

    #[tokio::test]
    async fn synced_lyrics_become_timed_lines() {
        let lines = lines(&Lyrics::Synced("[00:01.00]one\n[00:03.50]two".into()));
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].sections[0].content, "one");
        assert_eq!(lines[0].start_time, 1000);
        assert_eq!(lines[1].start_time, 3500);
    }

    #[tokio::test]
    async fn plain_lyrics_become_untimed_lines() {
        let lines = lines(&Lyrics::Plain("one\n\ntwo".into()));
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].sections[0].content, "two");
        assert_eq!((lines[1].start_time, lines[1].end_time), (0, 0));
    }

    #[tokio::test]
    async fn plain_lyrics_are_used_when_nothing_is_synced_and_instrumentals_are_told_so() {
        let http = Arc::new(FakeLrclib::default());
        http.answer(
            "/api/get",
            Some((
                200,
                r#"{"duration":200,"instrumental":false,"plainLyrics":"one\ntwo","syncedLyrics":null}"#,
            )),
        );
        assert_eq!(
            find(&http.lrclib(), &song(200)).await.unwrap(),
            Some(Lyrics::Plain("one\ntwo".into()))
        );
        http.answer(
            "/api/get",
            Some((
                200,
                r#"{"duration":200,"instrumental":true,"plainLyrics":"la","syncedLyrics":"[00:01.00]la"}"#,
            )),
        );
        assert_eq!(
            find(&http.lrclib(), &song(200)).await.unwrap(),
            Some(Lyrics::Instrumental)
        );
        assert!(lines(&Lyrics::Instrumental).is_empty());
    }

    #[tokio::test]
    async fn a_rate_limit_is_an_error_that_names_the_wait() {
        let http = Arc::new(FakeLrclib::default());
        http.answer("/api/get", Some((429, "")));
        let error = find(&http.lrclib(), &song(200)).await.unwrap_err();
        let limited = error.downcast_ref::<RateLimited>().unwrap();
        assert_eq!(limited.0, DEFAULT_RETRY_AFTER);
        assert_eq!(http.asked().len(), 1);
        let answer = HttpAnswer {
            status: 429,
            body: String::new(),
            retry_after: Some(7),
        };
        let error = refusal(&answer);
        assert_eq!(
            error.downcast_ref::<RateLimited>().unwrap().0,
            Duration::from_secs(7)
        );
    }

    #[tokio::test]
    async fn without_an_exact_match_the_search_gives_the_first_result_of_the_right_length() {
        let http = Arc::new(FakeLrclib::default());
        http.answer("/api/get", Some((404, "{}")));
        http.answer(
            "/api/search",
            Some((
                200,
                r#"[
                    {"trackName":"Night & Day","duration":260,"instrumental":false,"plainLyrics":"wrong length","syncedLyrics":null},
                    {"trackName":"Night & Day","duration":199,"instrumental":false,"plainLyrics":null,"syncedLyrics":null},
                    {"trackName":" night & DAY ","duration":201.5,"instrumental":false,"plainLyrics":"right","syncedLyrics":null},
                    {"trackName":"Night & Day","duration":200,"instrumental":false,"plainLyrics":"later","syncedLyrics":null}
                ]"#,
            )),
        );
        assert_eq!(
            find(&http.lrclib(), &song(200)).await.unwrap(),
            Some(Lyrics::Plain("right".into()))
        );
        let asked = http.asked();
        assert_eq!(asked.len(), 2);
        assert!(asked[1].starts_with("http://lrclib.test/api/search?track_name="));
        assert!(!asked[1].contains("duration"), "{asked:?}");
    }

    #[tokio::test]
    async fn nothing_anywhere_is_no_lyrics_and_a_failure_is_an_error() {
        let http = Arc::new(FakeLrclib::default());
        http.answer("/api/get", Some((404, "{}")));
        http.answer("/api/search", Some((200, "[]")));
        assert_eq!(find(&http.lrclib(), &song(200)).await.unwrap(), None);
        http.answer("/api/search", Some((500, "")));
        assert!(find(&http.lrclib(), &song(200)).await.is_err());
        http.answer("/api/get", Some((503, "")));
        assert!(find(&http.lrclib(), &song(200)).await.is_err());
        http.answer("/api/get", None);
        assert!(find(&http.lrclib(), &song(200)).await.is_err());
    }

    #[test]
    fn a_miss_is_asked_again_after_a_day_and_lyrics_never() {
        let cache = LyricsCache::default();
        cache.keep("a", None, 1000);
        cache.keep("b", Some(Lyrics::Plain("x".into())), 1000);
        assert!(matches!(cache.known("a", 1000 + 60), Known::Missing));
        assert!(matches!(
            cache.known("a", 1000 + MISS_RETRY_SECONDS),
            Known::Unknown
        ));
        assert!(matches!(
            cache.known("b", 1000 + 100 * MISS_RETRY_SECONDS),
            Known::Lyrics(_)
        ));
        assert!(matches!(cache.known("c", 1000), Known::Unknown));
    }

    #[tokio::test]
    async fn the_search_is_skipped_without_a_duration_and_takes_only_the_same_title() {
        let http = Arc::new(FakeLrclib::default());
        http.answer("/api/get", Some((404, "{}")));
        http.answer(
            "/api/search",
            Some((
                200,
                r#"[
                    {"trackName":"Another Song","duration":200,"instrumental":false,"plainLyrics":"other","syncedLyrics":null},
                    {"instrumental":false,"plainLyrics":"unnamed","syncedLyrics":null},
                    {"trackName":"NIGHT & day","duration":200,"instrumental":false,"plainLyrics":"same","syncedLyrics":null}
                ]"#,
            )),
        );
        let mut unknown_length = song(200);
        unknown_length.duration = None;
        assert_eq!(find(&http.lrclib(), &unknown_length).await.unwrap(), None);
        assert_eq!(http.asked().len(), 1);
        assert_eq!(
            find(&http.lrclib(), &song(200)).await.unwrap(),
            Some(Lyrics::Plain("same".into()))
        );
    }

    #[tokio::test]
    async fn a_search_result_without_a_length_is_not_taken_for_a_song_of_known_length() {
        let http = Arc::new(FakeLrclib::default());
        http.answer("/api/get", Some((404, "{}")));
        http.answer(
            "/api/search",
            Some((
                200,
                r#"[
                    {"trackName":"Night & Day","instrumental":false,"plainLyrics":"no length","syncedLyrics":null},
                    {"trackName":"Night & Day","duration":200,"instrumental":false,"plainLyrics":"right","syncedLyrics":null}
                ]"#,
            )),
        );
        assert_eq!(
            find(&http.lrclib(), &song(200)).await.unwrap(),
            Some(Lyrics::Plain("right".into()))
        );
    }

    #[tokio::test]
    async fn a_song_without_an_artist_or_title_is_not_looked_up() {
        let http = Arc::new(FakeLrclib::default());
        http.answer("/api/get", Some((200, SYNCED)));
        let mut nameless = song(200);
        nameless.artist = "  ".into();
        assert_eq!(find(&http.lrclib(), &nameless).await.unwrap(), None);
        nameless = song(200);
        nameless.title.clear();
        assert_eq!(find(&http.lrclib(), &nameless).await.unwrap(), None);
        assert!(http.asked().is_empty());
    }

    #[tokio::test]
    async fn a_bad_request_is_a_miss_not_an_error() {
        let http = Arc::new(FakeLrclib::default());
        http.answer("/api/get", Some((400, "{}")));
        http.answer("/api/search", Some((400, "{}")));
        assert_eq!(find(&http.lrclib(), &song(200)).await.unwrap(), None);
        http.answer("/api/get", Some((404, "{}")));
        assert_eq!(find(&http.lrclib(), &song(200)).await.unwrap(), None);
    }

    /// A clock that does not move.
    struct Still(Instant);

    impl Clock for Still {
        fn now(&self) -> Instant {
            self.0
        }

        fn system_time(&self) -> std::time::SystemTime {
            std::time::SystemTime::UNIX_EPOCH
        }
    }

    #[tokio::test]
    async fn lookups_stopped_while_the_answers_kept_are_read_ask_nothing() {
        use futures::FutureExt;

        let dir = tempfile::tempdir().unwrap();
        let http = Arc::new(FakeLrclib::default());
        http.answer("/api/get", Some((200, SYNCED)));
        let lyrics = LibraryLyrics::new(
            dir.path(),
            Uuid::new_v4(),
            http.lrclib(),
            Arc::new(Still(Instant::now())),
        );
        lyrics.set_allowed(true);
        // The answers kept are being read, as the first lookup reads them, until the test lets
        // the read end.
        let release = tokio::sync::Notify::new();
        let mut restore = Box::pin(lyrics.restored.get_or_init(|| release.notified()));
        assert!(restore.as_mut().now_or_never().is_none());
        let song = song(200);
        let mut lookup = Box::pin(lyrics.lyrics_of("a", &song));
        assert!(lookup.as_mut().now_or_never().is_none());
        // The user stops the lookups meanwhile.
        lyrics.set_allowed(false);
        release.notify_one();
        restore.await;
        assert_eq!(lookup.await, None);
        assert!(http.asked().is_empty());
    }

    #[test]
    fn a_save_writes_the_answers_kept_when_it_is_made() {
        let cache = LyricsCache::default();
        let saved = cache.keep("a", None, 1000);
        cache.keep("b", Some(Lyrics::Plain("x".into())), 1000);
        let json = serde_json::to_string(&saved).unwrap();
        assert!(json.contains("\"a\"") && json.contains("\"b\""), "{json}");
        let restored: HashMap<String, Kept> = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.len(), 2);
    }
}
