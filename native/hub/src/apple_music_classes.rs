//! What the catalog says of the albums an Apple Music library holds, kept on this device so an
//! artist's page is built at once from them and never waits on the catalog. The facts of the
//! albums are read in batches when the library's album list is kept (see
//! `AppleMusicBinding::albums`), or when a page finds albums without them. The views of an
//! artist are read when the artist's page is opened online and what is kept of them is missing
//! or old (see `VIEWS_TTL`).
//!
//! It is kept beside the binding's lists as `artist-classes` (see
//! `AppleMusicBinding::keep_beside_lists`), wrapped as every kept list is. The value:
//!
//! ```json
//! {
//!   "storefront": "it",
//!   "languages": { "it": ["it", "en-GB"] },
//!   "section_titles": { "it": { "en-GB": { "albums": "Albums", "singles": "Singles & EPs" } } },
//!   "albums": {
//!     "<catalog album id>": {
//!       "name": "Short EP - EP", "single": true, "compilation": null,
//!       "artists": ["<catalog artist id>"], "read": 1760000000
//!     }
//!   },
//!   "artist_views": {
//!     "it": {
//!       "<catalog artist id>": {
//!         "sections": { "<catalog album id>": "singles" }, "read": 1760000000
//!       }
//!     }
//!   }
//! }
//! ```
//!
//! - `albums`: per catalog album id, the facts Apple gives of it, not the section: the
//!   section depends on the artist page the album is shown on, so a page derives it (see
//!   `section_of`). An album the catalog does not know is kept with no artists and no flags,
//!   so it is not asked for again. `read` is seconds since the Unix epoch, by the binding's
//!   clock.
//! - `artist_views`: per storefront and catalog artist id, the section of the artist's page
//!   each release Apple lists in the artist's views is in (`ArtistSection::syntax`), by catalog
//!   album id. Apple sorts the releases by the kind the label reported, which the facts do not
//!   carry, so a release the views list goes where they put it, and the facts only place what
//!   they do not list. `read` is as for the albums.
//! - `languages`: the language tags each storefront supports, as Apple lists them.
//! - `section_titles`: the title Apple gives each section (`ArtistSection::syntax`), per
//!   storefront and supported language tag, as every artist there has the same. A page uses
//!   them only when they name every section it shows, and shows none otherwise.
//! - `storefront`: the storefront of the last read, whose titles a page uses.
//!
//! All of it is a cache of what Apple says: it goes with the account, and a missing entry only
//! means the page puts the album into Albums until it is read.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use apple_music::models::LibraryAlbum;
use database::mix_operator::ArtistSection;
use serde::{Deserialize, Serialize, Serializer};

use crate::apple_music_binding::AppleMusicBinding;

/// The name the classes are kept as beside the binding's lists.
const CLASSES: &str = "artist-classes";

/// How long the facts of an album count as young. A released album's facts rarely change, so
/// they are read again after a week, not as often as the lists.
pub(crate) const FACTS_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// How long what an artist's views say counts as young. Apple moves a release to another view
/// only when it comes out or its label reports it again, and a page shows only releases the
/// library already holds, so after a day the page is rarely behind Apple's; an artist opened
/// often is still read only once a day, which takes a request for every page of every view.
pub(crate) const VIEWS_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// How many artists' views are kept per storefront; those read longest ago go first.
const MAX_ARTIST_VIEWS: usize = 300;

/// What the catalog says of one album.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AlbumFacts {
    pub(crate) name: String,
    pub(crate) single: Option<bool>,
    pub(crate) compilation: Option<bool>,
    /// The catalog ids of the album's artists.
    pub(crate) artists: Vec<String>,
    pub(crate) read: u64,
}

/// What the views of a catalog artist's page say of its releases.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ArtistViews {
    /// The section each release the views list is in, by catalog album id, as the section's
    /// syntax.
    pub(crate) sections: HashMap<String, String>,
    pub(crate) read: u64,
}

/// The kept facts, views and titles (see the module docs).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Classes {
    #[serde(default)]
    pub(crate) storefront: Option<String>,
    #[serde(default)]
    pub(crate) languages: BTreeMap<String, Vec<String>>,
    /// The titles of the sections by storefront, language tag and the section's syntax.
    #[serde(default)]
    pub(crate) section_titles: BTreeMap<String, BTreeMap<String, BTreeMap<String, String>>>,
    #[serde(default)]
    pub(crate) albums: HashMap<String, AlbumFacts>,
    /// What the views of catalog artists say, by storefront and catalog artist id.
    #[serde(default)]
    pub(crate) artist_views: BTreeMap<String, HashMap<String, ArtistViews>>,
}

impl AlbumFacts {
    /// Whether the facts were read within `FACTS_TTL` before `now`, in seconds since the Unix
    /// epoch.
    pub(crate) fn is_fresh(&self, now: u64) -> bool {
        now.saturating_sub(self.read) < FACTS_TTL.as_secs()
    }
}

impl ArtistViews {
    /// Whether the views were read within `VIEWS_TTL` before `now`, in seconds since the Unix
    /// epoch.
    pub(crate) fn is_fresh(&self, now: u64) -> bool {
        now.saturating_sub(self.read) < VIEWS_TTL.as_secs()
    }

    /// The section the views put the album `id` (a catalog id) in; `None` when they do not
    /// list it.
    pub(crate) fn section(&self, id: &str) -> Option<ArtistSection> {
        self.sections
            .get(id)
            .and_then(|section| ArtistSection::from_syntax(section))
    }
}

impl Classes {
    /// What is kept of the views of the catalog artist `artist` in the storefront the last
    /// read was in.
    pub(crate) fn views_of(&self, artist: &str) -> Option<&ArtistViews> {
        self.artist_views
            .get(self.storefront.as_deref()?)?
            .get(artist)
    }

    /// Keeps `views` as what the views of the catalog artist `artist` say in `storefront`,
    /// and drops the artists read longest ago beyond `MAX_ARTIST_VIEWS`.
    pub(crate) fn keep_views(&mut self, storefront: &str, artist: &str, views: ArtistViews) {
        let kept = self.artist_views.entry(storefront.to_owned()).or_default();
        kept.insert(artist.to_owned(), views);
        while kept.len() > MAX_ARTIST_VIEWS {
            let Some(oldest) = kept
                .iter()
                .min_by_key(|(_, views)| views.read)
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            kept.remove(&oldest);
        }
    }

    /// The language tag of the storefront the last read was in that matches the client's
    /// `language`; `None` when the storefront or its languages are not kept yet, or none
    /// matches.
    pub(crate) fn tag_for(&self, language: &str) -> Option<&str> {
        let supported = self.languages.get(self.storefront.as_deref()?)?;
        matching_tag(language, supported)
    }

    /// The titles kept for the language `language` of the client in the storefront the last
    /// read was in.
    pub(crate) fn titles_for(&self, language: &str) -> Option<&BTreeMap<String, String>> {
        let tag = self.tag_for(language)?;
        self.section_titles
            .get(self.storefront.as_deref()?)?
            .get(tag)
    }

    /// Whether what a page shows of the facts differs from `before`: the facts without when
    /// they were read, and the titles.
    pub(crate) fn shown_differs(&self, before: &Self) -> bool {
        let strip = |classes: &Self| -> HashMap<String, AlbumFacts> {
            classes
                .albums
                .iter()
                .map(|(id, facts)| {
                    (
                        id.clone(),
                        AlbumFacts {
                            read: 0,
                            ..facts.clone()
                        },
                    )
                })
                .collect()
        };
        strip(self) != strip(before) || self.section_titles != before.section_titles
    }

    /// The catalog ids among `ids` whose facts are missing, or old at `now`, in seconds since
    /// the Unix epoch.
    pub(crate) fn needing<'a>(
        &self,
        ids: impl IntoIterator<Item = &'a String>,
        now: u64,
    ) -> Vec<String> {
        ids.into_iter()
            .filter(|id| {
                !self
                    .albums
                    .get(*id)
                    .is_some_and(|facts| facts.is_fresh(now))
            })
            .cloned()
            .collect()
    }
}

/// Whether a name is one Apple gives an EP or a single ("<title> - EP", "<title> - Single").
fn names_a_short_release(name: &str) -> bool {
    name.trim()
        .rsplit_once(['-', '\u{2013}', '\u{2014}'])
        .is_some_and(|(_, tail)| {
            let tail = tail.trim();
            tail.eq_ignore_ascii_case("ep") || tail.eq_ignore_ascii_case("single")
        })
}

/// The section of the page of the catalog artist `artist` that an album with `facts` is in:
/// Singles & EPs when Apple calls it a single or names it an EP or single, Compilations when
/// it is one, Appears On when its artists do not include `artist`, else Albums. Apple's album
/// attributes have no live flag, so live albums are Albums. An album without facts is Albums.
pub(crate) fn section_of(facts: Option<&AlbumFacts>, artist: &str) -> ArtistSection {
    let Some(facts) = facts else {
        return ArtistSection::Albums;
    };
    if facts.single == Some(true) || names_a_short_release(&facts.name) {
        ArtistSection::Singles
    } else if facts.compilation == Some(true) {
        ArtistSection::Compilations
    } else if !facts.artists.is_empty() && !facts.artists.iter().any(|id| id == artist) {
        ArtistSection::AppearsOn
    } else {
        ArtistSection::Albums
    }
}

/// The parts of a BCP 47 tag that matching looks at, lowercase: language, script, region, and
/// whether it carries anything else (a variant or extension, as `zh-Hant-NAN` does) or a
/// region Rune uses for a styled language (`en-EO`, Esperanto-styled English), which no
/// storefront has.
struct TagParts {
    language: String,
    script: Option<String>,
    region: Option<String>,
    styled: bool,
}

/// Regions Rune's own styled variants of a language carry; they are no countries.
const STYLED_REGIONS: [&str; 1] = ["eo"];

fn tag_parts(tag: &str) -> TagParts {
    let mut parts = tag.split(['-', '_']).map(str::to_lowercase);
    let mut found = TagParts {
        language: parts.next().unwrap_or_default(),
        script: None,
        region: None,
        styled: false,
    };
    for part in parts {
        let letters = part.chars().all(|c| c.is_ascii_alphabetic());
        if part.len() == 4 && letters && found.script.is_none() && found.region.is_none() {
            found.script = Some(part);
        } else if (part.len() == 2 && letters || part.len() == 3 && !letters)
            && found.region.is_none()
        {
            found.styled |= STYLED_REGIONS.contains(&part.as_str());
            found.region = Some(part);
        } else {
            found.styled = true;
        }
    }
    found
}

/// The tag of `supported` that answers in the client's `language`: an exact match first, then
/// the same language, preferring the same script and then the same region. A tag is never
/// matched by another that lacks a script, region or variant it carries, apart from a plain
/// region difference: `en` is `en-GB` and `zh-Hans` is `zh-Hans-CN`, but `zh-Hant` is not
/// `zh-Hans-CN`, and `ja-Kana`, `en-EO` and `zh-Hant-NAN` are not `ja`, `en-GB` and
/// `zh-Hant-TW`. `None` when no tag matches or the language is empty.
pub(crate) fn matching_tag<'a>(language: &str, supported: &'a [String]) -> Option<&'a str> {
    let language = language.trim();
    if language.is_empty() {
        return None;
    }
    if let Some(exact) = supported
        .iter()
        .find(|tag| tag.eq_ignore_ascii_case(language))
    {
        return Some(exact);
    }
    let wanted = tag_parts(language);
    if wanted.styled {
        return None;
    }
    let mut best: Option<(u8, &String)> = None;
    for tag in supported {
        let candidate = tag_parts(tag);
        if candidate.language != wanted.language
            || candidate.styled
            || (wanted.script.is_some() && wanted.script != candidate.script)
        {
            continue;
        }
        let score = u8::from(wanted.script.is_some()) * 2
            + u8::from(wanted.region.is_some() && wanted.region == candidate.region);
        if best.is_none_or(|(top, _)| score > top) {
            best = Some((score, tag));
        }
    }
    best.map(|(_, tag)| tag.as_str())
}

/// The classes as kept, with the count of the sign-outs that dropped them.
#[derive(Default)]
struct Kept {
    /// Counts the sign-outs that dropped the classes. A read that began under an earlier count
    /// is not kept.
    generation: u64,
    value: Arc<Classes>,
}

/// The binding's part of the classes: the classes kept, what reads them, and the album list
/// they were last asked for.
#[derive(Default)]
pub(crate) struct ClassStore {
    kept: Mutex<Kept>,
    /// Set once the classes kept on disk were read.
    restored: tokio::sync::OnceCell<()>,
    /// The catalog ids of the albums being read, and the keys the reads of section titles and
    /// of artists' views claim.
    running: Mutex<HashSet<String>>,
    /// The album list the facts were last asked for.
    listed: Mutex<Option<Arc<Vec<LibraryAlbum>>>>,
}

/// The classes the store holds, as they are kept beside the binding's lists.
struct Shared(Arc<Classes>);

impl Serialize for Shared {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ClassStore {
    /// The count to take before a read whose result changes the classes, and to hand back with
    /// it (see `change`).
    pub(crate) fn generation(&self) -> u64 {
        lock(&self.kept).generation
    }

    /// Forgets the classes, as the account signs out: what a read begun before brings is not
    /// kept. It comes before the binding's lists are cleared, so nothing is kept beside them
    /// after.
    pub(crate) fn clear(&self) {
        let mut kept = lock(&self.kept);
        kept.generation += 1;
        kept.value = Arc::default();
        *lock(&self.listed) = None;
    }

    /// The classes as kept, read from disk the first time they are asked for. Apple is never
    /// asked.
    pub(crate) async fn get(&self, binding: &AppleMusicBinding) -> Arc<Classes> {
        self.restored
            .get_or_init(|| async {
                let generation = self.generation();
                let Some(stored) = binding.kept_beside_lists::<Classes>(CLASSES).await else {
                    return;
                };
                let mut kept = lock(&self.kept);
                if kept.generation == generation && *kept.value == Classes::default() {
                    kept.value = Arc::new(stored);
                }
            })
            .await;
        Arc::clone(&lock(&self.kept).value)
    }

    /// Applies `change` to the classes and keeps them beside the binding's lists, unless the
    /// account signed out since `generation`. Answers whether it was applied.
    pub(crate) async fn change(
        &self,
        binding: &AppleMusicBinding,
        generation: u64,
        change: impl FnOnce(&mut Classes),
    ) -> bool {
        self.get(binding).await;
        let mut kept = lock(&self.kept);
        if kept.generation != generation {
            return false;
        }
        let mut next = Classes::clone(&kept.value);
        change(&mut next);
        kept.value = Arc::new(next);
        binding.keep_beside_lists(CLASSES, Shared(Arc::clone(&kept.value)));
        true
    }

    /// Marks `ids` as being read, and answers those that were not already.
    pub(crate) fn claim(&self, ids: Vec<String>) -> Vec<String> {
        let mut running = lock(&self.running);
        ids.into_iter()
            .filter(|id| running.insert(id.clone()))
            .collect()
    }

    /// Marks `ids` as read.
    pub(crate) fn release(&self, ids: &[String]) {
        let mut running = lock(&self.running);
        for id in ids {
            running.remove(id);
        }
    }

    /// Notes `albums` as the album list the facts were last asked for, and answers whether it
    /// is another one than the list before.
    pub(crate) fn listed_anew(&self, albums: &Arc<Vec<LibraryAlbum>>) -> bool {
        let mut listed = lock(&self.listed);
        if listed
            .as_ref()
            .is_some_and(|last| Arc::ptr_eq(last, albums))
        {
            return false;
        }
        *listed = Some(Arc::clone(albums));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{ArtistViews, Classes, MAX_ARTIST_VIEWS, matching_tag};
    use database::mix_operator::ArtistSection;

    fn tags(tags: &[&str]) -> Vec<String> {
        tags.iter().map(|tag| (*tag).to_owned()).collect()
    }

    #[test]
    fn the_client_language_is_matched_to_a_supported_tag() {
        let apple = tags(&[
            "en-US",
            "en-GB",
            "zh-Hans-CN",
            "zh-Hant-TW",
            "zh-Hant-HK",
            "ja",
            "pt-PT",
            "pt-BR",
        ]);
        assert_eq!(matching_tag("zh-Hans", &apple), Some("zh-Hans-CN"));
        assert_eq!(matching_tag("zh-Hant", &apple), Some("zh-Hant-TW"));
        assert_eq!(matching_tag("zh-Hant-HK", &apple), Some("zh-Hant-HK"));
        assert_eq!(matching_tag("en", &apple), Some("en-US"));
        assert_eq!(matching_tag("en-GB", &apple), Some("en-GB"));
        assert_eq!(matching_tag("ja", &apple), Some("ja"));
        assert_eq!(matching_tag("pt-BR", &apple), Some("pt-BR"));
        assert_eq!(matching_tag("pt-br", &apple), Some("pt-BR"));
        assert_eq!(matching_tag("", &apple), None);
        assert_eq!(matching_tag("fr", &apple), None);
    }

    #[test]
    fn a_script_never_matches_another_script() {
        assert_eq!(matching_tag("zh-Hant", &tags(&["zh-Hans-CN"])), None);
        assert_eq!(matching_tag("zh-Hans", &tags(&["it", "en-GB"])), None);
        assert_eq!(matching_tag("en", &tags(&["it", "en-GB"])), Some("en-GB"));
        assert_eq!(matching_tag("pt-BR", &tags(&["pt-PT"])), Some("pt-PT"));
        assert_eq!(matching_tag("en", &[]), None);
    }

    #[test]
    fn the_styled_variants_of_rune_match_nothing() {
        let apple = tags(&["en-GB", "en-US", "ja", "zh-Hans-CN", "zh-Hant-TW"]);
        for styled in [
            "en-EO",
            "ja-Kana",
            "ja-Nise",
            "zh-Hant-NAN",
            "zh-Hant-YUE",
            "zh-Hant-LZH",
        ] {
            assert_eq!(matching_tag(styled, &apple), None, "{styled}");
        }
    }

    #[test]
    fn the_views_of_the_artists_read_longest_ago_go_first() {
        let views = |read: u64, section: &str| ArtistViews {
            sections: [("4003".to_owned(), section.to_owned())].into(),
            read,
        };
        let mut classes = Classes {
            storefront: Some("it".to_owned()),
            ..Classes::default()
        };
        let total = u64::try_from(MAX_ARTIST_VIEWS).unwrap();
        for at in 0..total {
            classes.keep_views("it", &format!("{at}"), views(1000 + at, "singles"));
        }
        classes.keep_views("jp", "300", views(1, "albums"));
        classes.keep_views("it", "300", views(5000, "singles"));
        // The artist read first in Italy went, the one in Japan was kept apart.
        assert_eq!(classes.artist_views["it"].len(), MAX_ARTIST_VIEWS);
        assert!(classes.views_of("0").is_none());
        assert!(classes.views_of("1").is_some());
        assert_eq!(
            classes
                .views_of("300")
                .and_then(|views| views.section("4003")),
            Some(ArtistSection::Singles)
        );
        assert_eq!(
            classes
                .views_of("300")
                .and_then(|views| views.section("4002")),
            None
        );
        assert_eq!(classes.artist_views["jp"].len(), 1);
    }
}
