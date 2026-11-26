// Browsing an open Apple Music library through the requests the UI sends, against a fake
// Apple Music API.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result};
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use database::library_registry::{LibraryRecordId, ServiceKind};
use serde_json::{Value, json};
use tokio::net::TcpListener;

use crate::apple_music_binding::{AppleIdKind, AppleMusicBinding, CONTENT_CHANGED_DELAY, Tokens};
use crate::apple_music_translate::mix_collections::{RECENTLY_PLAYED_GROUP, UNTITLED_GROUP};
use crate::macros::{RequestSlots, capability_snapshot};
use crate::messages::{
    AddItemToMixRequest, AddItemToPlaylistRequest, CallerQueue, CapabilityReason,
    CollectionGroup, CollectionType, ComplexQuery, ComplexQueryRequest, CreatePlaylistRequest,
    ExcludedItems, FetchAllMixesRequest, FetchMixQueriesRequest, GetMixByIdRequest,
    RemoveMixRequest, UpdateMixRequest, FetchAllPlaylistsRequest,
    FetchCollectionByIdsRequest, FetchCollectionGroupSummaryRequest, FetchCollectionGroupsRequest,
    FetchMediaFileByIdsRequest, FetchMediaFilesRequest, FetchParsedMediaFileRequest,
    FetchPlaylistOccurrencesRequest, GetLyricByTrackIdRequest, GetLyricByTrackIdResponse,
    GetMediaFilesCountRequest, GetPlaylistByIdRequest, GetPrimaryColorByTrackIdRequest,
    LibraryContentChanged, LibraryHomeListMode, LibraryHomeQuery, LibraryTaskState,
    LibraryTaskStateChanged, MixQuery, MixQueryRequest, PlayingItemRef,
    RemoveItemFromPlaylistRequest, RemovePlaylistRequest, ReorderPlaylistItemPositionRequest,
    RequestCapability, RequestFailureCause, ResolveMixQueryRequest, SearchCollectionSummaryRequest,
    SearchMediaFileSummaryRequest, SetLikedRequest, UpdatePlaylistRequest,
};
use crate::node::NodeContext;
use crate::request_error::{ExpectedRequestError, classify, request_failure_cause};
use crate::rpc::{Slot, TargetRequest, UnsupportedReason, dispatch_gui_request};
use crate::server::http::websocket::withheld_library_event;
use crate::service::ServiceBinding;
use crate::service::covers::cover_cache_root;
use crate::service::folder::delete_service_library_folder;
use crate::service::lists::lists_folder;
use crate::service::lrclib::fake::FakeLrclib;
use crate::service::reach::Reach;
use crate::service::registry::{Clock, ServiceSetup, SystemClock};
use crate::test_wait::wait_until;

const USER_TOKEN: &str = "browse-user-token";
const DEVELOPER_TOKEN: &str = "browse-developer-token";
const APPLE_OTHER_TOKEN: &str = "browse-other-user-token";

/// What the fake Apple API saw and how it should answer.
struct AppleFake {
    base: String,
    /// Every request, as path and query, with whether it carried Apple credentials.
    seen: Mutex<Vec<(String, bool)>>,
    /// Answers 401 to every request except cover downloads.
    reject_everything: AtomicBool,
    /// Answers 401 to the next request that is not the storefront read.
    reject_next_read: AtomicBool,
    /// Answers every read of this path with this status and body, though the sign-in is fine.
    respond_path: Mutex<Option<(String, u16, String)>>,
    /// Holds the answer to every request whose path starts with this.
    delay_path: Mutex<Option<(String, std::time::Duration)>>,
    /// Lists the library as empty.
    empty: AtomicBool,
    /// Names the second album as the one added last.
    night_added_last: AtomicBool,
    /// The Night Album and its songs have no cover.
    bare_night: AtomicBool,
    more_artists: Mutex<Vec<Value>>,
    /// How many songs the library holds besides the three.
    extra_songs: usize,
    /// Holds the answer to the first read whose path starts with this, once, until the test
    /// releases it.
    hold_once: Mutex<Option<(String, Arc<tokio::sync::Notify>)>>,
    /// Lists "Artist One" credited alone and with "Guest" as the library's artists, and finds
    /// the catalog's "Guest" besides.
    credited: bool,
    /// Lists no song of the library or its playlists, though its albums and artists hold some.
    unlisted_songs: bool,
    /// Lists a song credited "Solo Voice", which has no artist relationship.
    solo: bool,
    /// Lists the songs of `catalog_credited_songs` too.
    catalog_credits: bool,
    /// How many reads of catalog songs are being answered, and the most there were at once.
    catalog_songs_in_flight: AtomicUsize,
    most_catalog_songs_in_flight: AtomicUsize,
    /// Whether the library is that of another account: the same, but every library id has
    /// "b-" after its kind (see `other_account_id`).
    other_account: AtomicBool,
    /// Every request that is not a read, as its method and target: Apple would take it as a
    /// change to the account.
    writes: Mutex<Vec<String>>,
    /// Lists the user's own playlist "Road Trip", whose songs join the song list, and the
    /// subscribed "Evening Jazz".
    playlists: bool,
    /// Answers the songs of `old_library_songs` when they are asked for by id, though the list
    /// lacks them. Cleared, Apple no longer has them.
    old_songs: AtomicBool,
    /// Recommends the catalog's albums and playlist (see `recommendations`); otherwise Apple
    /// recommends nothing.
    recommends: bool,
    /// The library songs the user rated liked on Apple, by their ids.
    liked: Mutex<HashSet<String>>,
    /// How many times the ratings were read.
    rating_reads: AtomicUsize,
    /// Fails every read of the ratings from the one with this number on, counting from 1; 0
    /// fails none.
    fail_rating_reads_from: AtomicUsize,
    /// Fails the first read of this path.
    fail_once: Mutex<Option<String>>,
    /// Lists releases of "Artist One" the catalog's page of it knows (see `kinds_albums`).
    kinds: bool,
    /// With `kinds`, also lists "Lost Tape", an album of "Artist One" no catalog view lists.
    lost_tape: bool,
    /// With `kinds`, also lists this many more albums of "Artist One", each with a catalog id
    /// the catalog does not know.
    many_albums: usize,
    /// Lists "Twin One" of "Twin Artist", on "Twin Second": the library's copy of a song of the
    /// second of the two albums the catalog has of the artist (see `catalog_artist`).
    twin: bool,
    /// Lists "Twin Third" among the albums of "Twin Artist" too, as a release that came out
    /// later.
    twin_new_album: AtomicBool,
    /// Gives the last page of the albums and of the top songs of "Viewed Artist" its own
    /// address as the next page, as a server that does not move on would.
    looping_views: AtomicBool,
    /// Gives the view of the albums "Viewed Artist" appears on no title, as Apple gives none
    /// for a view now and then.
    untitled_appears_on: AtomicBool,
}

/// The id another account's library has for `id`, when `id` is a library id: "i.s1" is
/// "i.b-s1". Catalog ids have none.
fn other_account_id(id: &str) -> Option<String> {
    let (kind, rest) = id.split_once('.')?;
    (matches!(kind, "i" | "l" | "p" | "r") && !rest.is_empty() && !rest.starts_with("b-"))
        .then(|| format!("{kind}.b-{rest}"))
}

/// Gives every library id in `value` as another account's library has it.
fn as_other_account(value: &mut Value) {
    match value {
        Value::String(text) => {
            if let Some(id) = other_account_id(text) {
                *text = id;
            }
        }
        Value::Array(items) => items.iter_mut().for_each(as_other_account),
        Value::Object(fields) => fields.values_mut().for_each(as_other_account),
        _ => {}
    }
}

/// Whether the query `query` asks for the relationship `relationship` to be included, as one
/// of the names its `include` lists.
fn includes(query: &str, relationship: &str) -> bool {
    url::form_urlencoded::parse(query.as_bytes()).any(|(name, value)| {
        name == "include" && value.split(',').any(|listed| listed == relationship)
    })
}

/// Counts a read being answered for as long as it lives.
struct InFlight<'a>(&'a AtomicUsize);

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl AppleFake {
    fn count(&self, prefix: &str) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(target, _)| target.starts_with(prefix))
            .count()
    }

    fn count_containing(&self, part: &str) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(target, _)| target.contains(part))
            .count()
    }

    /// How many requests whose target starts with `prefix` asked for the relationship
    /// `relationship` to be included.
    fn count_including(&self, prefix: &str, relationship: &str) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(target, _)| {
                target.starts_with(prefix)
                    && includes(target.split_once('?').map_or("", |(_, query)| query), relationship)
            })
            .count()
    }

    /// The ids each request whose target starts with `prefix` asked for, in order.
    fn asked_ids(&self, prefix: &str) -> Vec<Vec<String>> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(target, _)| target.starts_with(prefix))
            .map(|(target, _)| {
                let query = target.split_once('?').map_or("", |(_, query)| query);
                url::form_urlencoded::parse(query.as_bytes())
                    .filter(|(name, _)| name == "ids")
                    .flat_map(|(_, ids)| ids.split(',').map(str::to_owned).collect::<Vec<_>>())
                    .collect()
            })
            .collect()
    }

    /// How many requests Apple's API was asked. The covers the binding downloads in the
    /// background come from the same server, and are not what a test counts.
    fn total(&self) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(target, _)| !target.starts_with("/art/"))
            .count()
    }

    fn artwork(&self, name: &str) -> Value {
        json!({
            "width": 1400,
            "height": 1400,
            "url": format!("{}/art/{}/{{w}}x{{h}}bb.jpg", self.base, name.replace(' ', "_")),
        })
    }

    fn song(
        &self,
        id: &str,
        title: &str,
        album: (&str, &str),
        artist: (&str, &str),
        number: u32,
    ) -> Value {
        let mut song = json!({
            "id": id,
            "type": "library-songs",
            "href": format!("/v1/me/library/songs/{id}"),
            "attributes": {
                "name": title,
                "albumName": album.1,
                "artistName": artist.1,
                "trackNumber": number,
                "discNumber": 1,
                "durationInMillis": 200_000,
                "playParams": {"id": id, "kind": "song", "isLibrary": true, "catalogId": format!("c{id}")},
                "artwork": self.artwork(album.1),
            },
            "relationships": {
                "albums": {"data": [{"id": album.0, "type": "library-albums", "attributes": {"name": album.1}}]},
                "artists": {"data": [{"id": artist.0, "type": "library-artists", "attributes": {"name": artist.1}}]},
            }
        });
        if album.1 == "Night Album" && self.bare_night.load(Ordering::SeqCst) {
            song["attributes"]
                .as_object_mut()
                .expect("attributes")
                .remove("artwork");
        }
        song
    }

    fn album(&self, id: &str, name: &str, artist: (&str, &str)) -> Value {
        let mut album = json!({
            "id": id,
            "type": "library-albums",
            "href": format!("/v1/me/library/albums/{id}"),
            "attributes": {
                "name": name,
                "artistName": artist.1,
                "trackCount": 2,
                "artwork": self.artwork(name),
                "playParams": {"id": id, "kind": "album", "isLibrary": true},
            },
            "relationships": {
                "artists": {"data": [{"id": artist.0, "type": "library-artists", "attributes": {"name": artist.1}}]},
            }
        });
        if name == "Night Album" && self.bare_night.load(Ordering::SeqCst) {
            album["attributes"]
                .as_object_mut()
                .expect("attributes")
                .remove("artwork");
        }
        album
    }

    fn artist(id: &str, name: &str) -> Value {
        json!({
            "id": id,
            "type": "library-artists",
            "href": format!("/v1/me/library/artists/{id}"),
            "attributes": {"name": name},
        })
    }

    /// A library artist as listed with `include=catalog`, with the catalog artists Apple names
    /// for it and their pictures.
    fn credited_artist(&self, id: &str, name: &str, catalog: &[(&str, &str)]) -> Value {
        let mut artist = Self::artist(id, name);
        let catalog: Vec<Value> = catalog
            .iter()
            .map(|(id, name)| {
                json!({"id": id, "type": "artists", "attributes": {
                    "name": name,
                    "artwork": self.artwork(&format!("{name} Photo")),
                }})
            })
            .collect();
        artist["relationships"] = json!({"catalog": {"data": catalog}});
        artist
    }

    /// The library's artists when "Artist One" is credited alone and with "Guest": Apple
    /// names one catalog artist for one credit of two, and both for the other.
    fn credited_artists(&self) -> Vec<Value> {
        vec![
            self.credited_artist("r.art1", "Artist One", &[("300", "Artist One")]),
            self.credited_artist("r.duo", "Artist One & Guest", &[("300", "Artist One")]),
            self.credited_artist(
                "r.feat",
                "Guest, Artist One",
                &[("302", "Guest"), ("300", "Artist One")],
            ),
        ]
    }

    /// The albums and songs of the credits of "Artist One" with "Guest": "Duet" is on both
    /// of their albums, and "Guest, Artist One" lists both albums.
    fn credited_albums(&self) -> [(Value, Vec<Value>); 2] {
        let duo = ("r.duo", "Artist One & Guest");
        let feat = ("r.feat", "Guest, Artist One");
        let duet = self.song("i.d1", "Duet", ("l.duo", "Duo Album"), duo, 1);
        [
            (self.album("l.duo", "Duo Album", duo), vec![duet.clone()]),
            (
                self.album("l.feat", "Guest Album", feat),
                vec![
                    self.song("i.g1", "Guest Spot", ("l.feat", "Guest Album"), feat, 1),
                    duet,
                ],
            ),
        ]
    }

    /// A catalog song the fake knows: "ci.s1" is the catalog copy of the library's "i.s1".
    fn catalog_song(&self, id: &str) -> Option<Value> {
        if let Some(song) = self.viewed_song(id) {
            return Some(song);
        }
        let (title, album, artist, number) = match id {
            "777" => ("Catalog Tune", "Catalog Album", "Catalog Artist", None),
            "8001" => ("Unheld Hit", "Elsewhere", "Artist One", Some(1)),
            "ci.s1" => ("First Light", "Dawn Album", "Artist One", Some(1)),
            "ci.ab" => ("Together", "Shared Album", "Ann & Bob", Some(1)),
            "ci.ja" => ("Kana Song", "Shared Album", "Hiroyuki Sawano", Some(2)),
            "ci.en" => ("English Song", "Shared Album", "Hiroyuki Sawano", Some(3)),
            "ci.rev" => ("Reversed", "Shared Album", "Hiroyuki Sawano & Ann", Some(4)),
            "900" => ("Dawn Chorus", "Dawn Remixes", "Dawn Singer", Some(3)),
            "1601" => ("Remix One", "Dawn Remixes", "Dawn Singer", Some(1)),
            "1602" => ("Remix Two", "Dawn Remixes", "Dawn Singer", Some(2)),
            // Found by a search only.
            "1603" => ("Dawn Bonus", "Dawn Remixes", "Dawn Singer", Some(4)),
            "ci.b3" => ("Big Album 3", "Big Album", "Artist One", Some(3)),
            "ci.e2" => ("Short EP 2", "Short EP", "Artist One", Some(2)),
            "9501" => ("Early One", "Twin First", "Twin Artist", Some(1)),
            "9502" => ("Early Two", "Twin First", "Twin Artist", Some(2)),
            "ci.t1" => ("Twin One", "Twin Second", "Twin Artist", Some(1)),
            "9511" => ("Late Two", "Twin Second", "Twin Artist", Some(2)),
            "9521" => ("New One", "Twin Third", "Twin Artist", Some(1)),
            _ => return None,
        };
        // Its catalog artists, as Apple names them with `include=artists`.
        let artists: &[(&str, &str)] = match id {
            "ci.ab" => &[("400", "Ann"), ("401", "Bob")],
            "ci.ja" | "ci.en" => &[("156", "Hiroyuki Sawano")],
            "ci.rev" => &[("156", "Hiroyuki Sawano"), ("400", "Ann")],
            "ci.s1" | "8001" | "ci.b3" | "ci.e2" => &[("300", "Artist One")],
            "900" | "1601" | "1602" | "1603" => &[("301", "Dawn Singer")],
            "9501" | "9502" | "ci.t1" | "9511" | "9521" => &[("330", "Twin Artist")],
            _ => &[],
        };
        let artists: Vec<Value> = artists
            .iter()
            .map(|(id, name)| json!({"id": id, "type": "artists", "attributes": {"name": name}}))
            .collect();
        // Its catalog album, which Apple names for some.
        let album_id = match id {
            "1601" | "1602" | "1603" => Some("1600"),
            "8001" => Some("4007"),
            "9501" | "9502" => Some("1950"),
            "ci.t1" | "9511" => Some("1951"),
            "9521" => Some("1952"),
            _ => None,
        };
        let mut attributes = json!({
            "name": title,
            "albumName": album,
            "artistName": artist,
            "durationInMillis": 100_000,
            "playParams": {"id": id, "kind": "song"},
            "artwork": self.artwork(album),
        });
        if let Some(number) = number {
            attributes["trackNumber"] = json!(number);
            attributes["discNumber"] = json!(1);
        }
        let mut relationships = json!({"artists": {"data": artists}});
        if let Some(album_id) = album_id {
            relationships["albums"] = json!({"data": [{"id": album_id, "type": "albums"}]});
        }
        Some(json!({
            "id": id,
            "type": "songs",
            "attributes": attributes,
            "relationships": relationships,
        }))
    }

    /// The library's songs whose credits name the same artists otherwise: "Ann & Bob", two
    /// catalog artists; "サワノヒロユキ", "Hiroyuki Sawano" and "Sawano Hiroyuki & Ann", the same
    /// catalog artist in another script and name order; and "サワノヒロユキ & Cara", a song without a
    /// catalog id.
    fn catalog_credited_songs(&self) -> Vec<Value> {
        let album = ("l.alb3", "Shared Album");
        let song = |id: &str, title: &str, credit: &str, number| {
            self.song(id, title, album, (&format!("r.{id}"), credit), number)
        };
        let mut uncatalogued = song("i.nc", "No Catalog", "サワノヒロユキ & Cara", 5);
        uncatalogued["attributes"]["playParams"]
            .as_object_mut()
            .expect("play parameters")
            .remove("catalogId");
        vec![
            song("i.ab", "Together", "Ann & Bob", 1),
            song("i.ja", "Kana Song", "サワノヒロユキ", 2),
            song("i.en", "English Song", "Hiroyuki Sawano", 3),
            song("i.rev", "Reversed", "Sawano Hiroyuki & Ann", 4),
            uncatalogued,
        ]
    }

    /// `songs` as Apple answers a request that does not include their `relationship`.
    fn without(songs: &mut Value, relationship: &str) {
        for song in songs["data"].as_array_mut().into_iter().flatten() {
            if let Some(relationships) = song["relationships"].as_object_mut() {
                relationships.remove(relationship);
            }
        }
    }

    /// "i.copy", the library's copy of the catalog song "777", and "i.lost", a song of no catalog
    /// song, which the song list lacks (see `old_songs`).
    fn old_library_songs(&self) -> [Value; 2] {
        let old = |id: &str, title: &str| {
            self.song(
                id,
                title,
                ("l.old", "Old Album"),
                ("r.old", "Old Artist"),
                1,
            )
        };
        let mut copy = old("i.copy", "Copied Tune");
        copy["attributes"]["playParams"]["catalogId"] = json!("777");
        let mut lost = old("i.lost", "Lost Song");
        lost["attributes"]["playParams"]
            .as_object_mut()
            .expect("play parameters")
            .remove("catalogId");
        [copy, lost]
    }

    /// One of the user's library playlists, which the user can change when `editable`, and
    /// which follows a playlist of the catalog when `subscribed`.
    fn library_playlist(&self, id: &str, name: &str, editable: bool, subscribed: bool) -> Value {
        json!({
            "id": id,
            "type": "library-playlists",
            "href": format!("/v1/me/library/playlists/{id}"),
            "attributes": {
                "name": name,
                "canEdit": editable,
                "hasCatalog": subscribed,
                "isPublic": false,
                "playParams": {"id": id, "kind": "playlist", "isLibrary": true},
                "artwork": self.artwork(name),
            },
        })
    }

    /// A catalog album the fake knows, with the ids of its songs: "1500" is the catalog copy of
    /// the library's "Dawn Album".
    fn catalog_album(&self, id: &str) -> Option<(Value, &'static [&'static str])> {
        let (name, artist, songs): (_, _, &'static [&'static str]) = match id {
            "1500" => ("Dawn Album", ("300", "Artist One"), &["ci.s1"]),
            "1600" => ("Dawn Remixes", ("301", "Dawn Singer"), &["1602", "1601"]),
            "1900" => ("Guest Sessions", ("302", "Guest"), &[]),
            // The albums of "Twin Artist"; the library holds "Twin One" of the second.
            "1950" => ("Twin First", ("330", "Twin Artist"), &["9501", "9502"]),
            "1951" => ("Twin Second", ("330", "Twin Artist"), &["ci.t1", "9511"]),
            "1952" => ("Twin Third", ("330", "Twin Artist"), &["9521"]),
            // The releases of "Viewed Artist", whose page is read through `views`.
            "2001" => ("Alpha", ("310", "Viewed Artist"), &["2101", "2102"]),
            "2002" => ("Beta", ("310", "Viewed Artist"), &["2103"]),
            "2003" => ("Gamma", ("310", "Viewed Artist"), &["2104"]),
            "2010" => ("Spark - Single", ("310", "Viewed Artist"), &["2105"]),
            "2011" => ("Glow - EP", ("310", "Viewed Artist"), &["2106", "2107"]),
            "2012" => ("Odd Name", ("310", "Viewed Artist"), &["2109"]),
            "2020" => ("Others Record", ("311", "Another Artist"), &["2108"]),
            // The releases of "Artist One", whose page is read through `views` too.
            "4001" => ("First Album", ("300", "Artist One"), &[]),
            "4002" => ("Big Album", ("300", "Artist One"), &[]),
            "4003" => ("Short EP - EP", ("300", "Artist One"), &[]),
            "4004" => ("Lone Single - Single", ("300", "Artist One"), &[]),
            "4005" => ("Hits", ("300", "Artist One"), &[]),
            "4006" => ("Guest Record", ("302", "Guest"), &[]),
            // The album of "Unheld Hit", a song of "Artist One" on another artist's album.
            "4007" => ("Elsewhere", ("302", "Guest"), &["8001"]),
            _ => return None,
        };
        // Whether Apple calls it a single: the EP named so does, and the one without a name
        // that says so does not.
        let single = match id {
            "2010" | "2011" | "4003" | "4004" => json!(true),
            "2012" => json!(false),
            _ => Value::Null,
        };
        let compilation = if id == "4005" { json!(true) } else { Value::Null };
        // The albums of "Viewed Artist" came out in these years; the others have no date.
        let released = match id {
            "2001" => json!("2019-03-01"),
            "2002" => json!("2021-06-01"),
            "2003" => json!("2019-11-01"),
            _ => Value::Null,
        };
        let album = json!({
            "id": id,
            "type": "albums",
            "attributes": {
                "name": name,
                "artistName": artist.1,
                "trackCount": songs.len(),
                "isSingle": single,
                "isCompilation": compilation,
                "releaseDate": released,
                "artwork": self.artwork(name),
                "playParams": {"id": id, "kind": "album"},
            },
            "relationships": {"artists": {"data": [{"id": artist.0, "type": "artists"}]}},
        });
        Some((album, songs))
    }

    /// A catalog artist the fake knows, with the ids of their albums: "300" has the name of
    /// the library's "Artist One", and "303" is another artist of that name.
    fn catalog_artist(id: &str) -> Option<(Value, &'static [&'static str])> {
        let (name, albums): (_, &'static [&'static str]) = match id {
            "300" => ("Artist One", &["1500"]),
            "303" => ("Artist One", &[]),
            "301" => ("Dawn Singer", &["1600"]),
            "302" => ("Guest", &["1900"]),
            "330" => ("Twin Artist", &["1950", "1951"]),
            "310" => ("Viewed Artist", &[]),
            _ => return None,
        };
        let artist = json!({"id": id, "type": "artists", "attributes": {"name": name}});
        Some((artist, albums))
    }

    /// A song of "Viewed Artist": one of its releases' or one of its top songs.
    fn viewed_song(&self, id: &str) -> Option<Value> {
        if !(id.starts_with("21") || id.starts_with("30")) || id.len() != 4 {
            return None;
        }
        let number = id.parse::<u32>().ok()? % 100;
        Some(json!({
            "id": id,
            "type": "songs",
            "attributes": {
                "name": format!("Song {id}"),
                "albumName": "Viewed",
                "artistName": "Viewed Artist",
                "trackNumber": number,
                "discNumber": 1,
                "durationInMillis": 100_000,
                "playParams": {"id": id, "kind": "song"},
                "artwork": self.artwork("Viewed"),
            },
            "relationships": {"artists": {"data": [{"id": "310", "type": "artists"}]}},
        }))
    }

    /// A page of a view of the catalog artist `artist`: the albums or songs `ids`, with the
    /// subpath of its next page at `next` when there is one.
    fn view_page(
        &self,
        artist: &str,
        view: &str,
        ids: &[&str],
        songs: bool,
        next: Option<u32>,
    ) -> Value {
        let data: Vec<Value> = ids
            .iter()
            .map(|id| {
                if songs {
                    self.catalog_song(id).expect("a catalog song")
                } else {
                    self.catalog_album(id).expect("a catalog album").0
                }
            })
            .collect();
        let mut page = json!({"data": data});
        if let Some(offset) = next {
            page["next"] =
                json!(format!("/v1/catalog/it/artists/{artist}/view/{view}?offset={offset}"));
        }
        page
    }

    /// A view's first page under the title Apple gives the view.
    fn titled(mut page: Value, title: &str) -> Value {
        page["attributes"] = json!({"title": title});
        page
    }

    /// The page with the titles of its views in the language `l` of the query: the fake writes
    /// them in English, and Italy answers in Italian unless `l=en-GB` is sent.
    fn in_language(mut page: Value, query: &str) -> Value {
        let english = url::form_urlencoded::parse(query.as_bytes())
            .any(|(name, tag)| name == "l" && tag == "en-GB");
        if english {
            return page;
        }
        let italian = |title: &str| -> String {
            match title {
                "Latest Release" => "Ultima uscita",
                "Albums" => "Album",
                "Singles & EPs" => "Singoli ed EP",
                "Essential Albums" => "Album essenziali",
                "Compilations" => "Raccolte",
                "Appears On" => "Compare in",
                other => other,
            }
            .to_owned()
        };
        if let Some(views) = page
            .pointer_mut("/data/0/views")
            .and_then(Value::as_object_mut)
        {
            for view in views.values_mut() {
                if let Some(title) = view["attributes"]["title"].as_str().map(italian) {
                    view["attributes"]["title"] = json!(title);
                }
            }
        }
        page
    }

    /// What the fake answers about "Viewed Artist" with `views`, and for the pages its views
    /// continue on: the albums on two pages, the singles and EPs together, the albums it
    /// appears on, and five top songs on three pages, the first of which holds the library's
    /// song under its catalog copy.
    fn viewed_artist(&self, rest: &str, query: &str) -> Option<Value> {
        let page = |view: &str, ids: &[&str], songs: bool, next: Option<u32>| {
            self.view_page("310", view, ids, songs, next)
        };
        let offset = url::form_urlencoded::parse(query.as_bytes())
            .find(|(name, _)| name == "offset")
            .and_then(|(_, offset)| offset.parse::<u32>().ok());
        // The last pages name themselves as the next one when the views loop.
        let looping = self.looping_views.load(Ordering::SeqCst);
        let appears_on = page("appears-on-albums", &["2020"], false, None);
        let appears_on = if self.untitled_appears_on.load(Ordering::SeqCst) {
            appears_on
        } else {
            Self::titled(appears_on, "Appears On")
        };
        match rest {
            "310" if query.contains("views=") => Some(json!({"data": [{
                "id": "310",
                "type": "artists",
                "attributes": {"name": "Viewed Artist"},
                "views": {
                    "latest-release": Self::titled(page("latest-release", &["2011"], false, None), "Latest Release"),
                    "full-albums": Self::titled(page("full-albums", &["2001", "2002"], false, Some(2)), "Albums"),
                    "singles": Self::titled(page("singles", &["2010", "2011", "2012"], false, None), "Singles & EPs"),
                    "featured-albums": Self::titled(page("featured-albums", &["2001", "2002"], false, None), "Essential Albums"),
                    "compilation-albums": json!({"data": []}),
                    "appears-on-albums": appears_on,
                    "top-songs": page("top-songs", &["ci.s1", "3001"], true, Some(2)),
                },
            }]})),
            "310/view/full-albums" if offset == Some(2) => Some(page(
                "full-albums",
                &["2003"],
                false,
                looping.then_some(2),
            )),
            "310/view/top-songs" if offset == Some(2) => {
                Some(page("top-songs", &["3002", "3003"], true, Some(4)))
            }
            "310/view/top-songs" if offset == Some(4) => {
                Some(page("top-songs", &["3004"], true, looping.then_some(4)))
            }
            _ => None,
        }
    }

    /// The library releases of "Artist One" with their songs, each naming the catalog album it
    /// is a copy of (see `kinds_artist`): "Big Album" has seven songs, "Short EP" four, "Lone
    /// Single" one, "Hits" one and is by "Various Artists". With `lost_tape`, "Lost Tape" has
    /// one song and names a catalog album no view lists.
    fn kinds_albums(&self) -> Vec<(Value, Vec<Value>)> {
        let artist = ("r.art1", "Artist One");
        let release =
            |id: &str, name: &str, by: (&str, &str), year: &str, catalog: &str, ids: &[&str]| {
                let mut album = self.album(id, name, by);
                album["attributes"]["trackCount"] = json!(ids.len());
                album["attributes"]["releaseDate"] = json!(format!("{year}-01-01"));
                album["attributes"]["playParams"]["catalogId"] = json!(catalog);
                let songs = ids
                    .iter()
                    .enumerate()
                    .map(|(at, song)| {
                        let mut song = self.song(
                            song,
                            &format!("{name} {}", at + 1),
                            (id, name),
                            artist,
                            u32::try_from(at + 1).expect("a small number"),
                        );
                        song["attributes"]["releaseDate"] = json!(format!("{year}-01-01"));
                        song
                    })
                    .collect();
                (album, songs)
            };
        let mut albums = vec![
            release(
                "l.big",
                "Big Album",
                artist,
                "2022",
                "4002",
                &["i.b1", "i.b2", "i.b3", "i.b4", "i.b5", "i.b6", "i.b7"],
            ),
            release("l.sgl", "Lone Single", artist, "2023", "4004", &["i.g1x"]),
            release(
                "l.ep",
                "Short EP",
                artist,
                "2021",
                "4003",
                &["i.e1", "i.e2", "i.e3", "i.e4"],
            ),
            release(
                "l.va",
                "Hits",
                ("r.va", "Various Artists"),
                "2018",
                "4005",
                &["i.v1"],
            ),
        ];
        if self.lost_tape {
            albums.push(release("l.lost", "Lost Tape", artist, "2019", "4999", &["i.l1"]));
        }
        albums
    }

    /// What the fake answers about "Artist One" (catalog "300") when `kinds` is set, with
    /// `views`, and for the pages its views continue on. The albums are "First Album" and
    /// "Big Album" on two pages, the singles are "Lone Single" and the EP "Short EP", "Hits"
    /// is a compilation, "Guest Record" is on another artist's album, and the top songs are on
    /// two pages: only "ci.e2", "ci.s1" and "ci.b3" are copies of the library's songs.
    fn kinds_artist(&self, rest: &str, query: &str) -> Option<Value> {
        let page = |view: &str, ids: &[&str], songs: bool, next: Option<u32>| {
            self.view_page("300", view, ids, songs, next)
        };
        let offset = url::form_urlencoded::parse(query.as_bytes())
            .find(|(name, _)| name == "offset")
            .and_then(|(_, offset)| offset.parse::<u32>().ok());
        match rest {
            "300" if query.contains("views=") => Some(json!({"data": [{
                "id": "300",
                "type": "artists",
                "attributes": {"name": "Artist One"},
                "views": {
                    "latest-release": Self::titled(page("latest-release", &["4002"], false, None), "Latest Release"),
                    "full-albums": Self::titled(page("full-albums", &["4001"], false, Some(1)), "Albums"),
                    "singles": Self::titled(page("singles", &["4004", "4003"], false, None), "Singles & EPs"),
                    "featured-albums": Self::titled(page("featured-albums", &["4002", "4001"], false, None), "Essential Albums"),
                    "compilation-albums": Self::titled(page("compilation-albums", &["4005"], false, None), "Compilations"),
                    "appears-on-albums": Self::titled(page("appears-on-albums", &["4006"], false, Some(1)), "Appears On"),
                    "top-songs": page("top-songs", &["ci.e2", "8001"], true, Some(2)),
                },
            }]})),
            "300/view/full-albums" if offset == Some(1) => {
                Some(page("full-albums", &["4002"], false, None))
            }
            "300/view/appears-on-albums" if offset == Some(1) => {
                Some(page("appears-on-albums", &[], false, None))
            }
            "300/view/top-songs" if offset == Some(2) => {
                Some(page("top-songs", &["ci.s1", "ci.b3"], true, None))
            }
            _ => None,
        }
    }

    /// "Twin One", the library's copy of the catalog's "ci.t1" on "Twin Second".
    fn twin_song(&self) -> Value {
        let mut song = self.song(
            "i.t1",
            "Twin One",
            ("l.twin", "Twin Second"),
            ("r.twin", "Twin Artist"),
            1,
        );
        song["attributes"]["playParams"]["catalogId"] = json!("ci.t1");
        song
    }

    /// What the fake answers under `/v1/catalog/it/artists/`: the pages of "Viewed Artist" and,
    /// with `kinds`, of "Artist One", read through their views; the albums of "Twin Artist" on
    /// two pages, one each; and the other catalog artists with their albums.
    fn artists_answer(&self, rest: &str, query: &str) -> Option<Value> {
        if let Some(viewed) = self.viewed_artist(rest, query) {
            return Some(Self::in_language(viewed, query));
        }
        if self.kinds
            && let Some(viewed) = self.kinds_artist(rest, query)
        {
            return Some(Self::in_language(viewed, query));
        }
        if rest == "330/albums" {
            let second = url::form_urlencoded::parse(query.as_bytes())
                .any(|(name, offset)| name == "offset" && offset == "1");
            return Some(if second {
                let mut albums = vec![self.catalog_album("1951")?.0];
                if self.twin_new_album.load(Ordering::SeqCst) {
                    albums.push(self.catalog_album("1952")?.0);
                }
                json!({"data": albums})
            } else {
                json!({
                    "data": [self.catalog_album("1950")?.0],
                    "next": "/v1/catalog/it/artists/330/albums?offset=1",
                })
            });
        }
        // A page of a view of "Viewed Artist" or "Artist One" is had only at the offset its
        // previous page names.
        if rest.starts_with("310/view/") || self.kinds && rest.starts_with("300/view/") {
            return None;
        }
        if rest.contains("/view/") {
            return Some(json!({"data": []}));
        }
        let (id, albums) = match rest.strip_suffix("/albums") {
            Some(id) => (id, true),
            None => (rest, false),
        };
        let (artist, ids) = Self::catalog_artist(id)?;
        Some(if albums {
            json!({"data": ids
                .iter()
                .filter_map(|id| self.catalog_album(id).map(|(album, _)| album))
                .collect::<Vec<_>>()})
        } else {
            json!({"data": [artist]})
        })
    }

    /// The catalog albums asked for by id, each with its songs when they are asked to be
    /// included.
    fn catalog_albums(&self, query: &str) -> Value {
        let ids = url::form_urlencoded::parse(query.as_bytes())
            .find(|(name, _)| name == "ids")
            .map(|(_, ids)| ids.into_owned())
            .unwrap_or_default();
        let tracks = includes(query, "tracks");
        json!({"data": ids
            .split(',')
            .filter_map(|id| {
                let (mut album, songs) = self.catalog_album(id)?;
                if tracks {
                    album["relationships"]["tracks"] = self.catalog_songs(songs);
                }
                Some(album)
            })
            .collect::<Vec<_>>()})
    }

    /// The catalog's playlist "Morning Picks".
    fn catalog_playlist(&self) -> Value {
        json!({
            "id": "pl.cat1",
            "type": "playlists",
            "attributes": {
                "name": "Morning Picks",
                "curatorName": "Apple Music",
                "artwork": self.artwork("Morning Picks"),
                "playParams": {"id": "pl.cat1", "kind": "playlist"},
            },
        })
    }

    /// The user's recommendations: "Made for You" twice, which merge without listing their
    /// common album twice, an album also in recent plays, a group without a title of each kind,
    /// one of stations only, and a playlist without a name, alone in a group too.
    fn recommendations(&self) -> Value {
        let album = |id: &str, name: &str| {
            json!({"id": id, "type": "albums", "attributes": {
                "name": name, "artistName": "Dawn Singer", "artwork": self.artwork(name),
            }})
        };
        let picks = json!({"id": "pl.cat1", "type": "playlists", "attributes": {
            "name": "Morning Picks", "curatorName": "Apple Music",
            "artwork": self.artwork("Morning Picks"),
        }});
        let station = json!({"id": "ra.1", "type": "stations", "attributes": {"name": "Radio"}});
        // A playlist a user shared, which Apple gives without a name.
        let gone = json!({"id": "pl.u-gone", "type": "playlists", "attributes": {"name": ""}});
        let group = |id: &str, kind: &str, title: Option<&str>, contents: Vec<Value>| {
            let mut attributes = json!({"kind": kind});
            if let Some(title) = title {
                attributes["title"] = json!({"stringForDisplay": title});
            }
            json!({
                "id": id,
                "type": "personal-recommendation",
                "attributes": attributes,
                "relationships": {"contents": {"data": contents}},
            })
        };
        let (made, recent) = ("music-recommendations", "recently-played");
        json!({"data": [
            group(
                "rec-1",
                made,
                Some("Made for You"),
                vec![picks.clone(), album("1600", "Dawn Remixes"), station.clone()],
            ),
            group(
                "rec-2",
                recent,
                Some("Recently Played"),
                vec![album("1600", "Dawn Remixes"), album("1500", "Dawn Album")],
            ),
            group(
                "rec-3",
                made,
                Some("Made for You"),
                vec![album("1600", "Dawn Remixes"), album("1500", "Dawn Album")],
            ),
            group("rec-4", made, None, vec![picks.clone()]),
            group("rec-5", made, Some("Stations"), vec![station]),
            group("rec-6", recent, None, vec![picks, gone.clone()]),
            // A group of only that playlist.
            group("rec-7", made, Some("Gone"), vec![gone]),
        ]})
    }

    fn catalog_songs(&self, ids: &[&str]) -> Value {
        json!({"data": ids.iter().filter_map(|id| self.catalog_song(id)).collect::<Vec<_>>()})
    }

    fn songs(&self) -> [Value; 3] {
        [
            self.song(
                "i.s1",
                "First Light",
                ("l.alb1", "Dawn Album"),
                ("r.art1", "Artist One"),
                1,
            ),
            self.song(
                "i.s2",
                "Second Wind",
                ("l.alb1", "Dawn Album"),
                ("r.art1", "Artist One"),
                2,
            ),
            self.song(
                "i.s3",
                "Third Road",
                ("l.alb2", "Night Album"),
                ("r.art2", "Artist Two"),
                1,
            ),
        ]
    }

    fn answer(&self, path: &str, query: &str) -> Option<Value> {
        if self.empty.load(Ordering::SeqCst)
            && matches!(
                path,
                "/v1/me/library/songs" | "/v1/me/library/albums" | "/v1/me/library/artists"
            )
        {
            return Some(json!({"data": []}));
        }
        let [s1, s2, s3] = self.songs();
        let albums = [
            self.album("l.alb1", "Dawn Album", ("r.art1", "Artist One")),
            self.album("l.alb2", "Night Album", ("r.art2", "Artist Two")),
        ];
        Some(match path {
            "/v1/me/storefront" => {
                json!({"data": [{"id": "it", "type": "storefronts", "attributes": {"name": "Italy"}}]})
            }
            // Italy answers in Italian, or in British English when asked.
            "/v1/storefronts/it" => json!({"data": [{
                "id": "it", "type": "storefronts",
                "attributes": {
                    "name": "Italy",
                    "defaultLanguageTag": "it",
                    "supportedLanguageTags": ["it", "en-GB"],
                },
            }]}),
            "/v1/me/library/songs" if self.unlisted_songs && !query.contains("ids=") => {
                json!({"data": []})
            }
            "/v1/me/library/songs" => {
                let mut songs = vec![s1, s2, s3];
                if self.credited {
                    // The songs of the credits "Artist One" shares, one of each album.
                    let [(_, duo), (_, feat)] = self.credited_albums();
                    songs.extend([duo[0].clone(), feat[0].clone()]);
                }
                if self.catalog_credits {
                    songs.extend(self.catalog_credited_songs());
                }
                if self.twin {
                    songs.push(self.twin_song());
                }
                if self.kinds {
                    songs.extend(self.kinds_albums().into_iter().flat_map(|(_, songs)| songs));
                }
                if self.solo {
                    let mut solo = self.song(
                        "i.solo",
                        "Alone",
                        ("l.alb1", "Dawn Album"),
                        ("", "Solo Voice"),
                        7,
                    );
                    solo["relationships"]
                        .as_object_mut()
                        .expect("relationships")
                        .remove("artists");
                    songs.push(solo);
                }
                songs.extend((0..self.extra_songs).map(|n| {
                    self.song(
                        &format!("i.x{n:04}"),
                        &format!("Extra {n}"),
                        ("l.alb2", "Night Album"),
                        ("r.art2", "Artist Two"),
                        10,
                    )
                }));
                // Songs asked for by id are only those, and "i.s9" is one the list lacks.
                let asked = url::form_urlencoded::parse(query.as_bytes())
                    .find(|(name, _)| name == "ids")
                    .map(|(_, ids)| ids.split(',').map(str::to_owned).collect::<Vec<_>>());
                if let Some(asked) = asked {
                    songs.push(self.song(
                        "i.s9",
                        "Ninth Moon",
                        ("l.alb2", "Night Album"),
                        ("r.art2", "Artist Two"),
                        9,
                    ));
                    if self.old_songs.load(Ordering::SeqCst) {
                        songs.extend(self.old_library_songs());
                    }
                    songs.retain(|song| asked.iter().any(|id| song["id"] == id.as_str()));
                }
                json!({"data": songs})
            }
            "/v1/catalog/it/songs" => {
                let ids = url::form_urlencoded::parse(query.as_bytes())
                    .find(|(name, _)| name == "ids")
                    .map(|(_, ids)| ids.into_owned())
                    .unwrap_or_default();
                let mut songs = self.catalog_songs(&ids.split(',').collect::<Vec<_>>());
                // Asked to include their artists or albums, Apple names those of each song.
                for relationship in ["artists", "albums"] {
                    let included = url::form_urlencoded::parse(query.as_bytes())
                        .any(|(name, value)| name == "include" && value.contains(relationship));
                    if !included {
                        Self::without(&mut songs, relationship);
                    }
                }
                songs
            }
            "/v1/me/library/albums" => {
                let mut listed = albums.to_vec();
                if self.twin {
                    let mut twin = self.album("l.twin", "Twin Second", ("r.twin", "Twin Artist"));
                    twin["attributes"]["playParams"]["catalogId"] = json!("1951");
                    listed.push(twin);
                }
                if self.kinds {
                    // The page of "Artist One" holds the releases of `kinds_albums` alone.
                    listed.retain(|album| album["id"] != "l.alb1");
                    listed.extend(self.kinds_albums().into_iter().map(|(album, _)| album));
                    for at in 0..self.many_albums {
                        let mut album = self.album(
                            &format!("l.many{at}"),
                            &format!("Many {at}"),
                            ("r.art1", "Artist One"),
                        );
                        album["attributes"]["playParams"]["catalogId"] =
                            json!(format!("{}", 7000 + at));
                        listed.push(album);
                    }
                }
                json!({"data": listed})
            }
            "/v1/me/library/recently-added" if self.night_added_last.load(Ordering::SeqCst) => {
                json!({"data": [albums[1]]})
            }
            "/v1/me/library/recently-added" => json!({"data": []}),
            "/v1/me/library/albums/l.alb1" => json!({"data": [albums[0]]}),
            "/v1/me/library/albums/l.alb2" => json!({"data": [albums[1]]}),
            "/v1/me/library/albums/l.alb1/tracks" => json!({"data": [s1, s2]}),
            "/v1/me/library/albums/l.alb2/tracks" => json!({"data": [s3]}),
            "/v1/me/library/artists" if self.credited => json!({"data": self.credited_artists()}),
            "/v1/me/library/artists" => {
                let mut artists = vec![
                    Self::artist("r.art1", "Artist One"),
                    Self::artist("r.art2", "Artist Two"),
                ];
                artists.extend(self.more_artists.lock().unwrap().iter().cloned());
                json!({"data": artists})
            }
            "/v1/me/library/artists/r.duo" => {
                json!({"data": [Self::artist("r.duo", "Artist One & Guest")]})
            }
            "/v1/me/library/artists/r.feat" => {
                json!({"data": [Self::artist("r.feat", "Guest, Artist One")]})
            }
            "/v1/me/library/artists/r.duo/albums" => {
                let [(duo, _), _] = self.credited_albums();
                json!({"data": [duo]})
            }
            "/v1/me/library/artists/r.feat/albums" => {
                let [(duo, _), (feat, _)] = self.credited_albums();
                json!({"data": [feat, duo]})
            }
            credited
                if credited.starts_with("/v1/me/library/albums/l.duo")
                    || credited.starts_with("/v1/me/library/albums/l.feat") =>
            {
                let [duo, feat] = self.credited_albums();
                let (album, songs) = if credited.contains("l.duo") {
                    duo
                } else {
                    feat
                };
                if credited.ends_with("/tracks") {
                    json!({"data": songs})
                } else {
                    json!({"data": [album]})
                }
            }
            // Asked with `include=catalog`, Apple names the catalog artist of the library's.
            "/v1/me/library/artists/r.art1" => {
                let mut artist = Self::artist("r.art1", "Artist One");
                artist["relationships"] =
                    json!({"catalog": {"data": [{"id": "300", "type": "artists"}]}});
                json!({"data": [artist]})
            }
            "/v1/me/library/artists/r.art2" => {
                let mut artist = Self::artist("r.art2", "Artist Two");
                artist["relationships"] = json!({"catalog": {"data": []}});
                json!({"data": [artist]})
            }
            "/v1/me/library/artists/r.art1/albums" => json!({"data": [albums[0]]}),
            // The song list holds the songs of the user's own playlists too.
            "/v1/me/library/playlists" if self.playlists => json!({"data": [
                self.library_playlist("p.mine1", "Road Trip", true, false),
                self.library_playlist("p.mine2", "Sunday", true, false),
                self.library_playlist("p.fav", "Favourite Songs", false, false),
                self.library_playlist("p.sub1", "Evening Jazz", false, true),
            ]}),
            "/v1/me/library/playlists" => json!({"data": []}),
            "/v1/me/library/playlists/p.mine1/tracks" if self.playlists => {
                json!({"data": [s1, s3]})
            }
            "/v1/me/library/playlists/p.mine2/tracks" if self.playlists => {
                json!({"data": [s3, s2]})
            }
            "/v1/me/library/playlists/p.fav/tracks" if self.playlists => json!({"data": [s2]}),
            "/v1/me/library/playlists/p.sub1/tracks" if self.playlists => {
                json!({"data": [s2, s1]})
            }
            "/v1/me/ratings/library-songs" => {
                let liked = self.liked.lock().unwrap();
                let rated: Vec<Value> = url::form_urlencoded::parse(query.as_bytes())
                    .filter(|(name, _)| name == "ids")
                    .flat_map(|(_, ids)| ids.split(',').map(str::to_owned).collect::<Vec<_>>())
                    .filter(|id| liked.contains(id))
                    .map(|id| json!({"id": id, "type": "ratings", "attributes": {"value": 1}}))
                    .collect();
                json!({"data": rated})
            }
            // Every search finds the same: in the library, the first song, album and artist; in
            // the catalog, the copies of those and what the library lacks. A search names no
            // album or artist of a song it found.
            "/v1/me/library/search" => {
                let mut songs = json!({"data": [s1.clone()]});
                Self::without(&mut songs, "albums");
                Self::without(&mut songs, "artists");
                json!({"results": {
                    "library-songs": songs,
                    "library-albums": {"data": [albums[0]]},
                    "library-artists": {"data": [Self::artist("r.art1", "Artist One")]},
                }})
            }
            "/v1/catalog/it/search" => {
                let mut songs = self.catalog_songs(&["ci.s1", "900", "1603"]);
                Self::without(&mut songs, "albums");
                Self::without(&mut songs, "artists");
                let album = |id| self.catalog_album(id).map(|(album, _)| album);
                let artist = |id: &str, name: &str| {
                    json!({"id": id, "type": "artists", "attributes": {"name": name}})
                };
                json!({"results": {
                    "songs": songs,
                    "albums": {"data": [album("1500"), album("1600")]},
                    "artists": {"data": [artist("300", "Artist One"), artist("301", "Dawn Singer")]},
                }})
            }
            "/v1/me/recommendations" if self.recommends => self.recommendations(),
            "/v1/me/recommendations" => json!({"data": []}),
            "/v1/catalog/it/playlists/pl.cat1" => json!({"data": [self.catalog_playlist()]}),
            "/v1/catalog/it/playlists/pl.cat1/tracks" => self.catalog_songs(&["900", "777"]),
            "/v1/catalog/it/albums" => self.catalog_albums(query),
            catalog if catalog.starts_with("/v1/catalog/it/artists/") => {
                self.artists_answer(&catalog["/v1/catalog/it/artists/".len()..], query)?
            }
            catalog if catalog.starts_with("/v1/catalog/it/albums/") => {
                let rest = &catalog["/v1/catalog/it/albums/".len()..];
                let (id, tracks) = match rest.strip_suffix("/tracks") {
                    Some(id) => (id, true),
                    None => (rest, false),
                };
                let (album, songs) = self.catalog_album(id)?;
                if tracks {
                    self.catalog_songs(songs)
                } else {
                    json!({"data": [album]})
                }
            }
            _ => return None,
        })
    }
}

async fn apple_fake(
    State(fake): State<Arc<AppleFake>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let authed = headers.contains_key("authorization") || headers.contains_key("music-user-token");
    let target = uri
        .path_and_query()
        .map(|target| target.as_str().to_owned())
        .unwrap_or_default();
    if method != Method::GET {
        fake.writes
            .lock()
            .unwrap()
            .push(format!("{method} {target}"));
    }
    fake.seen.lock().unwrap().push((target, authed));
    let path = uri.path();
    let _in_flight = path.starts_with("/v1/catalog/it/songs").then(|| {
        let now = fake.catalog_songs_in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        fake.most_catalog_songs_in_flight
            .fetch_max(now, Ordering::SeqCst);
        InFlight(&fake.catalog_songs_in_flight)
    });
    let hold = {
        let mut hold = fake.hold_once.lock().unwrap();
        if method == Method::GET
            && hold
                .as_ref()
                .is_some_and(|(prefix, _)| path.starts_with(prefix.as_str()))
        {
            hold.take()
        } else {
            None
        }
    };
    if let Some((_, release)) = hold {
        release.notified().await;
    }
    let delay = fake
        .delay_path
        .lock()
        .unwrap()
        .clone()
        .filter(|(prefix, _)| path.starts_with(prefix.as_str()));
    if let Some((_, delay)) = delay {
        tokio::time::sleep(delay).await;
    }
    if path.starts_with("/art/") {
        let size: usize = if path.contains("/Huge/") {
            8 * 1024 * 1024 + 1
        } else {
            11
        };
        let body = if size == 11 {
            b"cover-bytes".to_vec()
        } else {
            vec![7; size]
        };
        return (StatusCode::OK, body).into_response();
    }
    let answered = fake
        .respond_path
        .lock()
        .unwrap()
        .clone()
        .filter(|(answered, _, _)| answered == path);
    if let Some((_, status, body)) = answered {
        return (StatusCode::from_u16(status).unwrap(), body).into_response();
    }
    let rejected = fake.reject_everything.load(Ordering::SeqCst)
        || (path != "/v1/me/storefront" && fake.reject_next_read.swap(false, Ordering::SeqCst));
    if rejected {
        return (
            StatusCode::UNAUTHORIZED,
            r#"{"errors":[{"status":"401","code":"UNAUTHORIZED"}]}"#,
        )
            .into_response();
    }
    let failed_once = {
        let mut failing = fake.fail_once.lock().unwrap();
        failing.as_deref() == Some(path) && failing.take().is_some()
    };
    let failed_rating = path == "/v1/me/ratings/library-songs" && {
        let read = fake.rating_reads.fetch_add(1, Ordering::SeqCst) + 1;
        let from = fake.fail_rating_reads_from.load(Ordering::SeqCst);
        from != 0 && read >= from
    };
    if failed_once || failed_rating {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"errors":[{"status":"500","code":"50000"}]}"#,
        )
            .into_response();
    }
    // Nothing writes to the Apple account.
    if method != Method::GET {
        return (StatusCode::METHOD_NOT_ALLOWED, "{}").into_response();
    }
    // Another account's library answers with its own ids, and knows only those.
    let other = fake.other_account.load(Ordering::SeqCst);
    let own_id = |text: &str| {
        if other {
            text.replace(".b-", ".")
        } else {
            text.to_owned()
        }
    };
    let query = uri.query().unwrap_or_default();
    let asks_own_ids = other && path.starts_with("/v1/me/") && {
        let ids = url::form_urlencoded::parse(query.as_bytes())
            .filter(|(name, _)| name == "ids")
            .flat_map(|(_, ids)| ids.split(',').map(str::to_owned).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        ids.iter().any(|id| other_account_id(id).is_some())
    };
    let names_own_id = other && path.split('/').any(|part| other_account_id(part).is_some());
    if asks_own_ids || names_own_id {
        return (StatusCode::NOT_FOUND, "{}").into_response();
    }
    match fake.answer(&own_id(path), &own_id(query)) {
        Some(mut body) => {
            if other && path.starts_with("/v1/me/") {
                as_other_account(&mut body);
            }
            (StatusCode::OK, body.to_string()).into_response()
        }
        None => (StatusCode::NOT_FOUND, "{}").into_response(),
    }
}

/// A clock a test moves on by hand, so what the libraries keep ages without waiting.
struct TestClock {
    start: Instant,
    /// The time of day at `start`. The time of day moves on from it with `now`, so a step of the
    /// host's clock does not move it.
    day_start: std::time::SystemTime,
    passed: Mutex<std::time::Duration>,
}

impl TestClock {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            start: Instant::now(),
            day_start: std::time::SystemTime::now(),
            passed: Mutex::default(),
        })
    }

    /// Moves the clock on by `by`.
    fn advance(&self, by: std::time::Duration) {
        *self.passed.lock().unwrap() += by;
    }
}

impl Clock for TestClock {
    fn now(&self) -> Instant {
        Instant::now().max(self.start) + *self.passed.lock().unwrap()
    }

    fn system_time(&self) -> std::time::SystemTime {
        self.day_start + self.now().saturating_duration_since(self.start)
    }
}

/// How a node's services reach the fake at `base`, look lyrics up at `lrclib` and tell the time
/// by `clock`.
fn fake_services(base: &str, clock: &Arc<TestClock>, lrclib: &Arc<FakeLrclib>) -> ServiceSetup {
    ServiceSetup {
        apple_music_host: Some(base.to_owned()),
        clock: Arc::clone(clock) as Arc<dyn Clock>,
        lyrics: lrclib.lrclib(),
    }
}

/// A node on the configuration in `config` whose services reach the fake at `base`, as one
/// started again on it.
async fn node_on(
    config: &str,
    broadcaster: Arc<dyn crate::utils::Broadcaster>,
    base: &str,
) -> Result<Arc<NodeContext>> {
    NodeContext::with_reconnect_config(
        config,
        config,
        broadcaster,
        false,
        RunMode::App,
        ReconnectConfig::default(),
        crate::session::PlayerSession::real_player(),
        Arc::new(::visualizer::Visualizer::new()),
        fake_services(base, &TestClock::new(), &Arc::default()),
    )
    .await
}

/// An Apple Music library signed in against the fake and open in the UI.
struct OpenApple {
    fixture: Fixture,
    node: Arc<NodeContext>,
    binding: Arc<AppleMusicBinding>,
    fake: Arc<AppleFake>,
    record: LibraryRecordId,
    server: tokio::task::JoinHandle<()>,
    /// The clock what the library keeps ages by.
    clock: Arc<TestClock>,
    /// The lyrics database the library looks lyrics up at, which fails to connect until it is
    /// told how to answer.
    lrclib: Arc<FakeLrclib>,
    /// The UI params the open made. The node keeps them weakly, and in the app the GUI request
    /// listeners keep them alive. In a test run another test's listeners take the shared signal
    /// channels over and end these, so the test holds the params itself.
    ui: Arc<crate::utils::GlobalParams>,
}

async fn open_apple(dir: &Path) -> Result<OpenApple> {
    open_apple_with(dir, AppleOptions::default()).await
}

/// Like `open_apple`, for a library with nothing in it when `empty` is set.
async fn open_apple_listing(dir: &Path, empty: bool) -> Result<OpenApple> {
    open_apple_with(
        dir,
        AppleOptions {
            empty,
            ..AppleOptions::default()
        },
    )
    .await
}

/// How the fake Apple library is set up before it is opened.
#[derive(Default)]
struct AppleOptions {
    empty: bool,
    extra_songs: usize,
    /// What the node's broadcaster is told, if the test looks.
    heard: Option<Arc<Heard>>,
    /// Whether the Night Album and its songs come without a cover.
    bare_night: bool,
    /// Whether "Artist One" is credited with "Guest" too.
    credited: bool,
    /// Whether the song list is empty while the albums and artists still hold songs.
    unlisted_songs: bool,
    /// Whether a song credited "Solo Voice" has no artist relationship.
    solo: bool,
    /// Whether the songs of `catalog_credited_songs` are listed too.
    catalog_credits: bool,
    /// Whether the user has playlists of their own in Apple Music (see `AppleFake::playlists`).
    playlists: bool,
    /// Whether Apple recommends anything (see `AppleFake::recommendations`).
    recommends: bool,
    /// The library songs the user rated liked on Apple.
    apple_likes: Vec<&'static str>,
    /// Holds Apple's first answer of the ratings until the test lets it go.
    hold_ratings: Option<Arc<tokio::sync::Notify>>,
    /// Holds Apple's first answer to a read whose path starts with this until the test lets it
    /// go, unless it holds the ratings.
    hold: Option<(&'static str, Arc<tokio::sync::Notify>)>,
    /// Fails the reads of the ratings from the one with this number on (see
    /// `AppleFake::fail_rating_reads_from`).
    fail_rating_reads_from: usize,
    /// Fails the first read of this path.
    fail_once: Option<&'static str>,
    /// Whether the open does not wait for the import the sign-in started to finish, as when the
    /// test holds or fails what it reads.
    import_unfinished: bool,
    /// Answers every read of this path with this status and body (see
    /// `AppleFake::respond_path`).
    respond: Option<(&'static str, u16, &'static str)>,
    /// Whether "Artist One" has the releases of `kinds_albums`.
    kinds: bool,
    /// Whether, with `kinds`, "Lost Tape" is among them.
    lost_tape: bool,
    /// How many more albums of "Artist One" the library lists with `kinds`.
    many_albums: usize,
    /// Whether the library holds "Twin One" of "Twin Artist".
    twin: bool,
}

async fn open_apple_with(dir: &Path, options: AppleOptions) -> Result<OpenApple> {
    use crate::messages::{RegisterServiceLibraryRequest, ServiceSignIn, StreamingService};
    use crate::utils::register_service_library_on_node;

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let clock = TestClock::new();
    let lrclib = Arc::new(FakeLrclib::default());
    let broadcaster: Arc<dyn crate::utils::Broadcaster> = match &options.heard {
        Some(heard) => heard.clone(),
        None => Arc::new(Quiet),
    };
    let fixture =
        super::fixture_with_services(dir, broadcaster, fake_services(&base, &clock, &lrclib))
            .await?;
    let node = fixture.global_params.node.clone();
    let fake = Arc::new(AppleFake {
        base: base.clone(),
        seen: Mutex::default(),
        reject_everything: AtomicBool::new(false),
        reject_next_read: AtomicBool::new(false),
        respond_path: Mutex::new(
            options
                .respond
                .map(|(path, status, body)| (path.to_owned(), status, body.to_owned())),
        ),
        delay_path: Mutex::default(),
        empty: AtomicBool::new(options.empty),
        night_added_last: AtomicBool::new(false),
        bare_night: AtomicBool::new(options.bare_night),
        more_artists: Mutex::default(),
        extra_songs: options.extra_songs,
        hold_once: Mutex::new(
            options
                .hold_ratings
                .clone()
                .map(|release| ("/v1/me/ratings/library-songs".to_owned(), release))
                .or_else(|| {
                    let (path, release) = options.hold.clone()?;
                    Some((path.to_owned(), release))
                }),
        ),
        credited: options.credited,
        unlisted_songs: options.unlisted_songs,
        solo: options.solo,
        catalog_credits: options.catalog_credits,
        catalog_songs_in_flight: AtomicUsize::new(0),
        most_catalog_songs_in_flight: AtomicUsize::new(0),
        other_account: AtomicBool::new(false),
        writes: Mutex::default(),
        playlists: options.playlists,
        old_songs: AtomicBool::new(false),
        recommends: options.recommends,
        liked: Mutex::new(
            options
                .apple_likes
                .iter()
                .map(|id| (*id).to_owned())
                .collect(),
        ),
        rating_reads: AtomicUsize::new(0),
        fail_rating_reads_from: AtomicUsize::new(options.fail_rating_reads_from),
        fail_once: Mutex::new(options.fail_once.map(str::to_owned)),
        kinds: options.kinds,
        lost_tape: options.lost_tape,
        many_albums: options.many_albums,
        twin: options.twin,
        twin_new_album: AtomicBool::new(false),
        looping_views: AtomicBool::new(false),
        untitled_appears_on: AtomicBool::new(false),
    });
    let app = Router::new().fallback(apple_fake).with_state(fake.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let registered = register_service_library_on_node(
        &RegisterServiceLibraryRequest {
            service: StreamingService::AppleMusic,
            alias: "Apple Music".into(),
            sign_in: ServiceSignIn::Platform {
                user_token: USER_TOKEN.into(),
                developer_token: Some(DEVELOPER_TOKEN.into()),
            },
            call_id: None,
        },
        &node,
    )
    .await?;
    let record = LibraryRecordId(
        registered
            .record
            .context("registered record")?
            .record_id
            .parse()?,
    );
    let binding = node
        .services
        .apple_music_binding(record)
        .context("signed-in binding")?;
    node.open_service_catalog(Some(record), binding.clone())
        .await?;
    let ui = node
        .current_ui()
        .await
        .context("UI params of the open library")?;
    wait_until("the song list to warm up", || async {
        Ok(binding.known_songs().map(|_| ()))
    })
    .await?;
    let apple = OpenApple {
        fixture,
        node,
        binding,
        fake,
        record,
        server,
        clock,
        lrclib,
        ui,
    };
    if !options.import_unfinished {
        import_ended(&apple, apple.record).await?;
    }
    Ok(apple)
}

/// The records folder of the Apple Music library `record`.
fn records_of(apple: &OpenApple, record: LibraryRecordId) -> std::path::PathBuf {
    use crate::service::device_records::device_records_folder;

    device_records_folder(Path::new(&*apple.node.config_path), record.0)
}

/// Waits for the first import of the library `record` to end: its playlists and likes are noted
/// as imported, the ids of its account are kept once the library holds any, and its binding
/// reads its records. A binding that takes over what a removed library left moves those files
/// into its folder before it lets them be read, so the files alone do not tell.
async fn import_ended(apple: &OpenApple, record: LibraryRecordId) -> Result<()> {
    let folder = records_of(apple, record);
    let binding = apple
        .node
        .services
        .apple_music_binding(record)
        .context("the binding of the library")?;
    let lists_songs = !apple.fake.empty.load(Ordering::SeqCst) && !apple.fake.unlisted_songs;
    let holds_any = lists_songs || apple.fake.playlists;
    wait_until("the import to end", || async {
        let ended = folder.join("playlists.imported").exists()
            && folder.join("likes.imported").exists()
            && (!holds_any || folder.join("account.json").exists())
            && !binding.records_held.load(Ordering::SeqCst);
        Ok(ended.then_some(()))
    })
    .await
}

impl OpenApple {
    async fn run<R>(&self, request: &R) -> Result<<R as crate::TypedRequest>::Response>
    where
        R: crate::rpc::TargetRequest
            + crate::Signal<
                Response = <R as crate::TypedRequest>::Response,
                Params = <R as crate::utils::ParamsExtractor>::Params,
            > + crate::utils::ParamsExtractor
            + rinf::DartSignal
            + serde::Serialize,
        <R as crate::TypedRequest>::Response: serde::de::DeserializeOwned,
    {
        dispatch_gui_request(&self.fixture.global_params, request)
            .await?
            .context("the request answered nothing")
    }

    async fn finish(self) -> Result<()> {
        self.server.abort();
        self.node.shutdown().await
    }
}

fn unsupported(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Unsupported { .. })
    )
}

fn mix(operator: &str, parameter: impl ToString) -> MixQueryRequest {
    MixQueryRequest {
        call_id: None,
        queries: vec![MixQuery {
            operator: operator.into(),
            parameter: parameter.to_string(),
        }],
        cursor: 0,
        page_size: 30,
        bake_cover_arts: false,
        queue: CallerQueue::default(),
        random_seed: None,
    }
}

#[test]
fn apple_music_slots_translate_browsing_and_leave_the_rest_unsupported() {
    let slots = RequestSlots::apple_music();
    for slot in [
        FetchMediaFilesRequest::slot(&slots),
        FetchMediaFileByIdsRequest::slot(&slots),
        FetchParsedMediaFileRequest::slot(&slots),
        SearchMediaFileSummaryRequest::slot(&slots),
        GetMediaFilesCountRequest::slot(&slots),
        FetchCollectionGroupSummaryRequest::slot(&slots),
        FetchCollectionGroupsRequest::slot(&slots),
        FetchCollectionByIdsRequest::slot(&slots),
        SearchCollectionSummaryRequest::slot(&slots),
        MixQueryRequest::slot(&slots),
        ResolveMixQueryRequest::slot(&slots),
        ComplexQueryRequest::slot(&slots),
        // Rune's own playlists, kept on this device.
        FetchAllPlaylistsRequest::slot(&slots),
        GetPlaylistByIdRequest::slot(&slots),
        CreatePlaylistRequest::slot(&slots),
        UpdatePlaylistRequest::slot(&slots),
        RemovePlaylistRequest::slot(&slots),
        AddItemToPlaylistRequest::slot(&slots),
        ReorderPlaylistItemPositionRequest::slot(&slots),
        RemoveItemFromPlaylistRequest::slot(&slots),
        FetchPlaylistOccurrencesRequest::slot(&slots),
        // Rune's own mixes, kept on this device, and the ones Apple recommends.
        FetchAllMixesRequest::slot(&slots),
        GetMixByIdRequest::slot(&slots),
        FetchMixQueriesRequest::slot(&slots),
        crate::messages::CreateMixRequest::slot(&slots),
        UpdateMixRequest::slot(&slots),
        RemoveMixRequest::slot(&slots),
        AddItemToMixRequest::slot(&slots),
        // The library and Apple's catalog, searched together.
        crate::messages::SearchForRequest::slot(&slots),
    ] {
        assert_eq!(slot, Slot::Translate);
    }
    // What is asked about one item is decided by the library of the item, which for an Apple
    // Music item is its color and whether it is liked in Rune (see
    // `RequestSlots::apple_music_items`).
    for slot in [
        GetPrimaryColorByTrackIdRequest::slot(&slots),
        GetLyricByTrackIdRequest::slot(&slots),
        SetLikedRequest::slot(&slots),
        crate::messages::GetLikedRequest::slot(&slots),
    ] {
        assert_eq!(slot, Slot::Native);
    }
    let items = RequestSlots::apple_music_items();
    assert_eq!(GetPrimaryColorByTrackIdRequest::slot(&items), Slot::Native);
    assert_eq!(SetLikedRequest::slot(&items), Slot::Translate);
    assert_eq!(
        crate::messages::GetLikedRequest::slot(&items),
        Slot::Translate
    );
    // The library looks its songs' lyrics up online, which only a library the user allowed to
    // has.
    assert_eq!(GetLyricByTrackIdRequest::slot(&items), Slot::Translate);
    let hidden = Slot::Unsupported {
        reason: UnsupportedReason::NotImplemented,
    };
    let allowed = crate::service::item_slots(ServiceKind::AppleMusic, true);
    assert_eq!(GetLyricByTrackIdRequest::slot(&allowed), Slot::Translate);
    let not_allowed = crate::service::item_slots(ServiceKind::AppleMusic, false);
    assert_eq!(GetLyricByTrackIdRequest::slot(&not_allowed), hidden);
    // What the library does not hold, its scans and playlists made of files, it does not
    // answer.
    for slot in [
        crate::messages::ScanAudioLibraryRequest::slot(&slots),
        crate::messages::CreateM3u8PlaylistRequest::slot(&slots),
    ] {
        assert_eq!(slot, hidden);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apple_music_capabilities_show_browsing_and_hide_the_rest() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let capabilities = capability_snapshot(&apple.node)?;
    for capability in [
        capabilities.fetch_media_files_request,
        capabilities.fetch_collection_group_summary_request,
        capabilities.mix_query_request,
        capabilities.complex_query_request,
        capabilities.resolve_mix_query_request,
        capabilities.get_primary_color_by_track_id_request,
        capabilities.get_lyric_by_track_id_request,
        capabilities.set_liked_request,
        capabilities.fetch_all_playlists_request,
        capabilities.create_playlist_request,
        capabilities.add_item_to_playlist_request,
        capabilities.fetch_playlist_occurrences_request,
        capabilities.fetch_all_mixes_request,
        capabilities.create_mix_request,
        capabilities.add_item_to_mix_request,
        capabilities.search_for_request,
    ] {
        assert_eq!(capability, RequestCapability::Available);
    }
    for capability in [
        capabilities.scan_audio_library_request,
        capabilities.get_analyze_count_request,
        capabilities.fetch_directory_tree_request,
        capabilities.create_m3u8_playlist_request,
    ] {
        assert_eq!(capability, RequestCapability::Hidden);
    }
    // The library stays on this device, so no other device can be cast to. The snapshot a peer
    // gets does not say so: it is a fact about this device's own UI.
    assert_ne!(
        capabilities.cast_resolved_items_request,
        RequestCapability::Disabled {
            reason: CapabilityReason::DeviceOnly,
        }
    );
    assert_eq!(
        apple.node.own_capabilities()?.cast_resolved_items_request,
        RequestCapability::Disabled {
            reason: CapabilityReason::DeviceOnly,
        }
    );
    apple.finish().await
}

/// What the Tracks page and the summaries say of whether each song plays, in order.
async fn song_availability(apple: &OpenApple) -> Result<Vec<(String, AudioAvailability)>> {
    let page = apple
        .run(&FetchMediaFilesRequest {
            call_id: None,
            cursor: 0,
            page_size: 50,
            bake_cover_arts: false,
        })
        .await?;
    let summaries = apple
        .run(&SearchMediaFileSummaryRequest {
            call_id: None,
            n: 50,
        })
        .await?;
    let listed: Vec<_> = page
        .media_files
        .into_iter()
        .map(|file| (file.track, file.audio_availability))
        .collect();
    assert_eq!(
        summaries
            .result
            .iter()
            .map(|summary| summary.audio_availability)
            .collect::<Vec<_>>(),
        listed
            .iter()
            .map(|(_, playable)| *playable)
            .collect::<Vec<_>>()
    );
    Ok(listed)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn offline_only_the_songs_stored_on_this_device_play_and_the_lists_reload() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (apple, heard) = open_apple_heard(dir.path()).await?;
    let all = |availability| -> Vec<(String, AudioAvailability)> {
        ["i.s1", "i.s2", "i.s3"]
            .into_iter()
            .map(|track| (track.to_owned(), availability))
            .collect()
    };
    // Online, and before the host says anything, every song plays from Apple.
    assert_eq!(
        song_availability(&apple).await?,
        all(AudioAvailability::RemoteOnly)
    );
    let announced = told_settled(&heard).await;

    // Offline, the downloaded song still plays and the others do not. A library id the
    // library lacks changes nothing for its songs.
    let downloaded = Reach::offline(HashSet::from(["i.s2".to_owned(), "i.other".to_owned()]));
    apple.node.services.reach_changed(
        crate::apple_music_binding::APPLE_MUSIC_HOST,
        downloaded.clone(),
    );
    assert_eq!(
        song_availability(&apple).await?,
        [
            ("i.s1".to_owned(), AudioAvailability::Unavailable),
            ("i.s2".to_owned(), AudioAvailability::RemoteOnly),
            ("i.s3".to_owned(), AudioAvailability::Unavailable),
        ]
    );
    assert_eq!(told_settled(&heard).await, announced + 1);
    // The same report again tells nobody.
    apple
        .node
        .services
        .reach_changed(crate::apple_music_binding::APPLE_MUSIC_HOST, downloaded);
    assert_eq!(told_settled(&heard).await, announced + 1);
    // A catalog song is never stored on this device.
    let catalog = "catalog:777";
    assert_eq!(
        apple.binding.audio_availability(catalog),
        AudioAvailability::Unavailable
    );
    assert_eq!(
        apple.binding.audio_availability("i.s2"),
        AudioAvailability::RemoteOnly
    );

    // A download that finishes meanwhile is a change.
    apple.node.services.reach_changed(
        crate::apple_music_binding::APPLE_MUSIC_HOST,
        Reach::offline(HashSet::from(["i.s1".to_owned()])),
    );
    assert_eq!(told_settled(&heard).await, announced + 2);

    // Online again, everything plays, and the lists reload.
    apple.node.services.reach_changed(
        crate::apple_music_binding::APPLE_MUSIC_HOST,
        Reach::default(),
    );
    assert_eq!(
        song_availability(&apple).await?,
        all(AudioAvailability::RemoteOnly)
    );
    assert_eq!(told_settled(&heard).await, announced + 3);
    assert_eq!(
        apple.binding.audio_availability(catalog),
        AudioAvailability::RemoteOnly
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_binding_made_while_offline_starts_offline() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (apple, _) = open_apple_heard(dir.path()).await?;
    apple.node.services.reach_changed(
        crate::apple_music_binding::APPLE_MUSIC_HOST,
        Reach::offline(HashSet::new()),
    );
    assert_eq!(
        apple.binding.audio_availability("i.s1"),
        AudioAvailability::Unavailable
    );
    // Signing in again keeps what the host reported.
    apple
        .binding
        .sign_in(Tokens::new(DEVELOPER_TOKEN, APPLE_OTHER_TOKEN).context("tokens")?);
    assert_eq!(
        apple.binding.audio_availability("i.s1"),
        AudioAvailability::Unavailable
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apple_music_songs_are_listed_fetched_by_id_and_counted() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let record_id = apple.record.0.to_string();
    // The import of the first sign-in read the song list as Apple has it.
    let read = apple.fake.count("/v1/me/library/songs");
    assert!(read >= 1);

    let page = apple
        .run(&FetchMediaFilesRequest {
            call_id: None,
            cursor: 1,
            page_size: 2,
            bake_cover_arts: false,
        })
        .await?;
    let titles: Vec<_> = page
        .media_files
        .iter()
        .map(|file| file.title.as_str())
        .collect();
    assert_eq!(titles, ["Second Wind", "Third Road"]);
    assert!(
        page.media_files
            .iter()
            .all(|file| file.library_id == record_id)
    );
    assert_eq!(page.media_files[0].track, "i.s2");
    assert_eq!(page.media_files[0].album, "Dawn Album");
    assert_eq!(page.media_files[0].track_number, Some(2));
    assert!(page.media_files[0].cover_art_id.is_some());

    let count = apple
        .run(&GetMediaFilesCountRequest { call_id: None })
        .await?;
    assert_eq!(count.count, 3);
    let summaries = apple
        .run(&SearchMediaFileSummaryRequest {
            call_id: None,
            n: 2,
        })
        .await?;
    assert_eq!(
        summaries
            .result
            .iter()
            .map(|song| song.name.as_str())
            .collect::<Vec<_>>(),
        ["First Light", "Second Wind"]
    );
    // All of this was answered from the song list kept, which was not read again.
    assert_eq!(apple.fake.count("/v1/me/library/songs"), read);

    // Ids of songs in the list, then one the list lacks and one only the catalog has.
    let by_ids = apple
        .run(&FetchMediaFileByIdsRequest {
            call_id: None,
            ids: vec![page.media_files[1].id, page.media_files[0].id],
            bake_cover_arts: false,
        })
        .await?;
    assert_eq!(by_ids.media_files[0].title, "Third Road");
    assert_eq!(by_ids.media_files[1].title, "Second Wind");
    assert_eq!(apple.fake.count("/v1/me/library/songs"), read);

    let library_miss = apple.binding.intern(AppleIdKind::LibrarySong, "i.s9")?;
    let catalog_miss = apple.binding.intern(AppleIdKind::CatalogSong, "777")?;
    let fetched = apple
        .run(&FetchMediaFileByIdsRequest {
            call_id: None,
            ids: vec![library_miss, catalog_miss],
            bake_cover_arts: false,
        })
        .await?;
    assert_eq!(fetched.media_files[0].title, "Ninth Moon");
    assert_eq!(fetched.media_files[0].track, "i.s9");
    assert_eq!(fetched.media_files[1].title, "Catalog Tune");
    assert_eq!(fetched.media_files[1].track, "catalog:777");
    assert_eq!(apple.fake.count("/v1/catalog/it/songs"), 1);
    // Asking again is answered from what was fetched.
    let before = apple.fake.total();
    apple
        .run(&FetchMediaFileByIdsRequest {
            call_id: None,
            ids: vec![library_miss, catalog_miss],
            bake_cover_arts: false,
        })
        .await?;
    assert_eq!(apple.fake.total(), before);

    let parsed = apple
        .run(&FetchParsedMediaFileRequest {
            call_id: None,
            id: page.media_files[0].id,
        })
        .await?;
    assert_eq!(parsed.file.title, "Second Wind");
    assert_eq!(parsed.album.name, "Dawn Album");
    assert_eq!(parsed.artists[0].name, "Artist One");
    assert_ne!(parsed.album.id, 0);

    // An id this binding never gave is an input error.
    let unknown = apple
        .run(&FetchMediaFileByIdsRequest {
            call_id: None,
            ids: vec![9999],
            bake_cover_arts: false,
        })
        .await
        .err()
        .context("unknown id")?;
    assert_eq!(
        classify(&unknown),
        crate::messages::RequestErrorKind::InvalidInput
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apple_music_collections_are_listed_grouped_and_fetched() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;

    for (collection_type, groups, names) in [
        (
            CollectionType::Album,
            vec![("D", 1), ("N", 1)],
            vec!["Dawn Album"],
        ),
        (
            CollectionType::Artist,
            vec![("A", 2)],
            vec!["Artist One", "Artist Two"],
        ),
    ] {
        let summary = apple
            .run(&FetchCollectionGroupSummaryRequest {
                call_id: None,
                collection_type,
            })
            .await?;
        let found: Vec<_> = summary
            .groups
            .iter()
            .map(|group| (group.group_title.as_str(), group.count))
            .collect();
        assert_eq!(found, groups, "{collection_type:?}");
        let title = groups[0].0.to_owned();
        let page = apple
            .run(&FetchCollectionGroupsRequest {
                call_id: None,
                collection_type,
                bake_cover_arts: false,
                group_titles: vec![title.clone()],
            })
            .await?;
        assert_eq!(page.groups[0].group_title, title);
        let listed: Vec<_> = page.groups[0]
            .collections
            .iter()
            .map(|collection| collection.name.as_str())
            .collect();
        assert_eq!(listed, names, "{collection_type:?}");
        let ids: Vec<i32> = page.groups[0].collections.iter().map(|c| c.id).collect();
        let by_ids = apple
            .run(&FetchCollectionByIdsRequest {
                call_id: None,
                collection_type,
                bake_cover_arts: false,
                ids: ids.clone(),
            })
            .await?;
        assert_eq!(
            by_ids.result.iter().map(|c| c.id).collect::<Vec<_>>(),
            ids,
            "{collection_type:?}"
        );
        assert!(
            by_ids
                .result
                .iter()
                .all(|c| c.collection_type == collection_type && c.readonly)
        );
    }

    let searched = apple
        .run(&SearchCollectionSummaryRequest {
            call_id: None,
            collection_type: Some(CollectionType::Album),
            bake_cover_arts: Some(false),
            n: 1,
        })
        .await?;
    assert_eq!(searched.result.len(), 1);

    // Genres and folders are refused when asked, not hidden by a panic.
    for refused in [CollectionType::Genre, CollectionType::Directory] {
        let error = apple
            .run(&FetchCollectionGroupsRequest {
                call_id: None,
                collection_type: refused,
                bake_cover_arts: false,
                group_titles: vec!["A".into()],
            })
            .await
            .err()
            .context("refused collection type")?;
        assert!(unsupported(&error), "{refused:?}");
    }
    // Ids are numbered per kind, so an id nothing was given for is an input error.
    let unknown = apple
        .run(&FetchCollectionByIdsRequest {
            call_id: None,
            collection_type: CollectionType::Album,
            bake_cover_arts: false,
            ids: vec![4242],
        })
        .await
        .err()
        .context("unknown collection id")?;
    assert_eq!(
        classify(&unknown),
        crate::messages::RequestErrorKind::InvalidInput
    );
    apple.finish().await
}

fn song_ref(apple: &OpenApple, track: &str) -> PlayingItemRef {
    PlayingItemRef::LibraryTrack {
        library_id: apple.record.0.to_string(),
        track: track.into(),
    }
}

fn resolve(
    apple: &OpenApple,
    album: i32,
    initial: &str,
    excluded: &[&str],
) -> ResolveMixQueryRequest {
    ResolveMixQueryRequest {
        queries: vec![MixQuery {
            operator: "lib::album".into(),
            parameter: album.to_string(),
        }],
        hint_position: 0,
        initial_playback_item: Some(song_ref(apple, initial)),
        instantly_play: true,
        fallback_playing_items: vec![song_ref(apple, "i.s3")],
        excluded: ExcludedItems {
            items: excluded.iter().map(|id| song_ref(apple, id)).collect(),
        },
        call_id: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apple_music_resolves_a_mix_query_in_order_without_the_queue() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let album = apple.binding.intern(AppleIdKind::Album, "l.alb1")?;
    let tracks = |response: &crate::messages::ResolveMixQueryResponse| {
        response
            .playing_items
            .iter()
            .map(|item| match item {
                PlayingItemRef::LibraryTrack { track, .. } => track.clone(),
                PlayingItemRef::IndependentFile { path } => path.clone(),
            })
            .collect::<Vec<_>>()
    };
    let all = apple.run(&resolve(&apple, album, "i.s2", &[])).await?;
    assert_eq!(tracks(&all), ["i.s1", "i.s2"]);
    assert_eq!(all.start_index, Some(1));
    // The caller's own queue is left out, and the start index follows.
    let without = apple
        .run(&resolve(&apple, album, "i.s2", &["i.s1"]))
        .await?;
    assert_eq!(tracks(&without), ["i.s2"]);
    assert_eq!(without.start_index, Some(0));
    // A track the query does not hold starts the fallback items at the hint.
    let mut elsewhere = resolve(&apple, album, "i.s9", &[]);
    elsewhere.hint_position = 1;
    let missing = apple.run(&elsewhere).await?;
    assert_eq!(tracks(&missing), ["i.s3"]);
    assert_eq!(missing.start_index, Some(1));
    // Without a wish to play there is no index.
    let mut quiet = resolve(&apple, album, "i.s2", &[]);
    quiet.instantly_play = false;
    let quiet = apple.run(&quiet).await?;
    assert_eq!(
        (tracks(&quiet), quiet.start_index),
        (vec!["i.s1".into(), "i.s2".into()], None)
    );
    // The song list answered all of this, with no read of the album.
    assert_eq!(apple.fake.count("/v1/me/library/albums/l.alb1"), 0);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apple_music_home_lists_its_groups_and_leaves_the_rest_empty() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let group = |title: &str, query| ComplexQuery {
        title: title.into(),
        query,
    };
    let mode = LibraryHomeListMode::Name;
    let response = apple
        .run(&ComplexQueryRequest {
            queries: vec![
                group("Artists", LibraryHomeQuery::Artists(mode)),
                group("Albums", LibraryHomeQuery::Albums(mode)),
                group("Playlists", LibraryHomeQuery::Playlists(mode)),
                group("Tracks", LibraryHomeQuery::Tracks(mode)),
                group("Liked", LibraryHomeQuery::Liked),
                group("Mixes", LibraryHomeQuery::Mixes(mode)),
                group("Genres", LibraryHomeQuery::Genres(mode)),
                group("Most", LibraryHomeQuery::Most),
            ],
            call_id: None,
        })
        .await?;
    let names = |index: usize| {
        response.result[index]
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        response
            .result
            .iter()
            .map(|group| group.title.as_str())
            .collect::<Vec<_>>(),
        [
            "Artists",
            "Albums",
            "Playlists",
            "Tracks",
            "Liked",
            "Mixes",
            "Genres",
            "Most"
        ]
    );
    assert_eq!(names(0), ["Artist One", "Artist Two"]);
    assert_eq!(names(1), ["Dawn Album", "Night Album"]);
    // Rune made no playlists here, and the library lists no mixes or likes.
    assert!(names(2).is_empty());
    assert_eq!(names(3), ["First Light", "Second Wind", "Third Road"]);
    for empty in 4..8 {
        assert!(response.result[empty].entries.is_empty());
    }
    // An album opens by the query that lists its tracks.
    let album = &response.result[1].entries[0];
    assert_eq!(album.queries[0].operator, "lib::album");
    assert_eq!(album.queries[0].parameter, album.id.to_string());
    assert!(album.readonly);
    // A track plays the listed tracks from itself.
    let track = &response.result[3].entries[1];
    let ids: Vec<_> = track
        .queries
        .iter()
        .map(|query| query.parameter.clone())
        .collect();
    assert_eq!(ids.len(), 3);
    assert_eq!(ids[0], track.id.to_string());
    // Those queries resolve to the tracks, in that order.
    let resolved = apple
        .run(&ResolveMixQueryRequest {
            queries: track.queries.clone(),
            hint_position: 0,
            initial_playback_item: None,
            instantly_play: false,
            fallback_playing_items: Vec::new(),
            excluded: ExcludedItems::default(),
            call_id: None,
        })
        .await?;
    assert_eq!(resolved.playing_items[0], song_ref(&apple, "i.s2"));
    assert_eq!(resolved.playing_items.len(), 3);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_tracks_page_asks_apple_for_the_songs_when_online() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let read = apple.fake.count("/v1/me/library/songs");
    let resolved = apple
        .run(&ResolveMixQueryRequest {
            queries: vec![MixQuery {
                operator: "lib::all".into(),
                parameter: "true".into(),
            }],
            hint_position: 0,
            initial_playback_item: Some(song_ref(&apple, "i.s2")),
            instantly_play: true,
            fallback_playing_items: Vec::new(),
            excluded: ExcludedItems::default(),
            call_id: None,
        })
        .await?;
    assert_eq!(resolved.start_index, Some(1));
    assert!(apple.fake.count("/v1/me/library/songs") > read);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apple_music_takes_the_queries_the_tracks_and_album_pages_send() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let album = apple.binding.intern(AppleIdKind::Album, "l.alb1")?;
    let resolve_with = |queries: Vec<MixQuery>, initial: &str| ResolveMixQueryRequest {
        queries,
        hint_position: 0,
        initial_playback_item: Some(song_ref(&apple, initial)),
        instantly_play: true,
        fallback_playing_items: Vec::new(),
        excluded: ExcludedItems::default(),
        call_id: None,
    };
    let keys = |response: &crate::messages::ResolveMixQueryResponse| {
        response
            .playing_items
            .iter()
            .map(|item| match item {
                PlayingItemRef::LibraryTrack { track, .. } => track.clone(),
                PlayingItemRef::IndependentFile { path } => path.clone(),
            })
            .collect::<Vec<_>>()
    };
    let query = |operator: &str, parameter: &str| MixQuery {
        operator: operator.into(),
        parameter: parameter.into(),
    };
    // The tracks page: every song, in the order of the song list.
    let all = vec![query("lib::all", "true")];
    let resolved = apple.run(&resolve_with(all.clone(), "i.s3")).await?;
    assert_eq!(keys(&resolved), ["i.s1", "i.s2", "i.s3"]);
    assert_eq!(resolved.start_index, Some(2));
    let page = apple
        .run(&MixQueryRequest {
            call_id: None,
            queries: all,
            cursor: 1,
            page_size: 5,
            bake_cover_arts: false,
            queue: CallerQueue::default(),
            random_seed: None,
        })
        .await?;
    assert_eq!(
        page.files
            .iter()
            .map(|file| file.track.as_str())
            .collect::<Vec<_>>(),
        ["i.s2", "i.s3"]
    );
    // An album's page: its tracks in order, with the track-number sort it already has.
    let detail = vec![
        query("lib::album", &album.to_string()),
        query("sort::track_number", "true"),
    ];
    let resolved = apple.run(&resolve_with(detail.clone(), "i.s2")).await?;
    assert_eq!(keys(&resolved), ["i.s1", "i.s2"]);
    let page = apple
        .run(&MixQueryRequest {
            call_id: None,
            queries: detail,
            cursor: 0,
            page_size: 5,
            bake_cover_arts: false,
            queue: CallerQueue::default(),
            random_seed: None,
        })
        .await?;
    assert_eq!(
        page.files
            .iter()
            .map(|file| file.track.as_str())
            .collect::<Vec<_>>(),
        ["i.s1", "i.s2"]
    );
    // A sort on anything but an album stays unsupported, and names the request.
    let refused = apple
        .run(&resolve_with(
            vec![
                query("lib::artist", "1"),
                query("sort::track_number", "true"),
            ],
            "i.s1",
        ))
        .await
        .err()
        .context("unsupported query")?;
    assert!(unsupported(&refused));
    assert!(format!("{refused:#}").contains("ResolveMixQueryRequest"));
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apple_music_answers_a_random_query_with_covers() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            bare_night: true,
            ..AppleOptions::default()
        },
    )
    .await?;
    let query = |count: &str, page: (i32, i32)| MixQueryRequest {
        call_id: None,
        queries: vec![
            MixQuery {
                operator: "lib::random".into(),
                parameter: count.into(),
            },
            MixQuery {
                operator: "filter::with_cover_art".into(),
                parameter: "true".into(),
            },
        ],
        cursor: page.0,
        page_size: page.1,
        bake_cover_arts: false,
        queue: CallerQueue::default(),
        random_seed: None,
    };
    let tracks = |response: &crate::messages::MixQueryResponse| {
        response
            .files
            .iter()
            .map(|file| file.track.clone())
            .collect::<Vec<_>>()
    };
    // Only the two songs with a cover are drawn, each once, for any count.
    let drawn = apple.run(&query("30", (0, 30))).await?;
    let mut sorted = tracks(&drawn);
    sorted.sort();
    assert_eq!(sorted, ["i.s1", "i.s2"]);
    // A smaller count is a sample of that size.
    assert_eq!(tracks(&apple.run(&query("1", (0, 30))).await?).len(), 1);
    // The page cuts the draw.
    assert_eq!(tracks(&apple.run(&query("30", (0, 1))).await?).len(), 1);
    assert!(tracks(&apple.run(&query("30", (2, 5))).await?).is_empty());
    // Without the filter the song without a cover is drawn too.
    let mut plain = mix("lib::random", 30);
    plain.page_size = 30;
    assert_eq!(tracks(&apple.run(&plain).await?).len(), 3);
    // The filter goes with the whole song list too.
    let mut album = mix("lib::all", "true");
    album.queries.push(MixQuery {
        operator: "filter::with_cover_art".into(),
        parameter: "true".into(),
    });
    assert_eq!(tracks(&apple.run(&album).await?), ["i.s1", "i.s2"]);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_pages_of_a_seeded_random_draw_are_pages_of_one_draw() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            extra_songs: 30,
            ..AppleOptions::default()
        },
    )
    .await?;
    let page = |seed: Option<u64>, cursor: i32, page_size: i32| {
        let mut request = mix("lib::random", 20);
        request.random_seed = seed;
        request.cursor = cursor;
        request.page_size = page_size;
        let apple = &apple;
        async move {
            Ok::<_, anyhow::Error>(
                apple
                    .run(&request)
                    .await?
                    .files
                    .into_iter()
                    .map(|file| file.track)
                    .collect::<Vec<_>>(),
            )
        }
    };
    // Two pages of a seeded draw are the two halves of the draw in one page.
    let whole = page(Some(7), 0, 20).await?;
    assert_eq!(whole.len(), 20);
    let mut halves = page(Some(7), 0, 10).await?;
    halves.extend(page(Some(7), 10, 10).await?);
    assert_eq!(halves, whole);
    // Another seed draws another list, as does a draw without a seed, almost surely.
    assert_ne!(page(Some(8), 0, 20).await?, whole);
    assert_ne!(page(None, 0, 20).await?, whole);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apple_music_random_query_of_an_empty_library_is_empty() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_listing(dir.path(), true).await?;
    let mut request = mix("lib::random", 30);
    request.queries.push(MixQuery {
        operator: "filter::with_cover_art".into(),
        parameter: "true".into(),
    });
    assert!(apple.run(&request).await?.files.is_empty());
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cover_filtered_collection_is_cut_from_the_song_list_for_every_page() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let album = apple.binding.intern(AppleIdKind::Album, "l.alb1")?;
    let page = |cursor| {
        let mut request = mix("lib::album", album);
        request.queries.push(MixQuery {
            operator: "filter::with_cover_art".into(),
            parameter: "true".into(),
        });
        request.cursor = cursor;
        request.page_size = 1;
        request
    };
    let first = apple.run(&page(0)).await?;
    assert_eq!(first.files.len(), 1);
    // A later page cuts the same songs, and the album is never read.
    let second = apple.run(&page(1)).await?;
    assert_eq!(second.files.len(), 1);
    assert_ne!(first.files[0].track, second.files[0].track);
    assert_eq!(apple.fake.count("/v1/me/library/albums/l.alb1"), 0);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apple_music_home_shows_the_newest_first_in_reverse() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    // Apple lists the album it added last first.
    apple.fake.night_added_last.store(true, Ordering::SeqCst);
    let group = |title: &str, query| ComplexQuery {
        title: title.into(),
        query,
    };
    let names = |response: &crate::messages::ComplexQueryResponse, index: usize| {
        response.result[index]
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect::<Vec<_>>()
    };
    let newest = apple
        .run(&ComplexQueryRequest {
            queries: vec![
                group(
                    "Albums",
                    LibraryHomeQuery::Albums(LibraryHomeListMode::Reverse),
                ),
                group(
                    "Tracks",
                    LibraryHomeQuery::Tracks(LibraryHomeListMode::Reverse),
                ),
            ],
            call_id: None,
        })
        .await?;
    assert_eq!(names(&newest, 0), ["Night Album", "Dawn Album"]);
    assert_eq!(
        names(&newest, 1),
        ["Third Road", "Second Wind", "First Light"]
    );
    let oldest = apple
        .run(&ComplexQueryRequest {
            queries: vec![
                group(
                    "Albums",
                    LibraryHomeQuery::Albums(LibraryHomeListMode::Forward),
                ),
                group(
                    "Tracks",
                    LibraryHomeQuery::Tracks(LibraryHomeListMode::Forward),
                ),
            ],
            call_id: None,
        })
        .await?;
    assert_eq!(names(&oldest, 0), ["Dawn Album", "Night Album"]);
    assert_eq!(
        names(&oldest, 1),
        ["First Light", "Second Wind", "Third Road"]
    );
    // A random group shows the same entries each time the home is read.
    let random = |_: ()| ComplexQueryRequest {
        queries: vec![group(
            "Tracks",
            LibraryHomeQuery::Tracks(LibraryHomeListMode::Random),
        )],
        call_id: None,
    };
    let first = apple.run(&random(())).await?;
    let second = apple.run(&random(())).await?;
    assert_eq!(names(&first, 0), names(&second, 0));
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_empty_apple_library_has_an_empty_home() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_listing(dir.path(), true).await?;
    let mode = LibraryHomeListMode::Reverse;
    let titles = ["Artists", "Albums", "Playlists", "Tracks", "Liked"];
    let queries = [
        LibraryHomeQuery::Artists(mode),
        LibraryHomeQuery::Albums(mode),
        LibraryHomeQuery::Playlists(mode),
        LibraryHomeQuery::Tracks(mode),
        LibraryHomeQuery::Liked,
    ];
    let response = apple
        .run(&ComplexQueryRequest {
            queries: titles
                .iter()
                .zip(queries)
                .map(|(title, query)| ComplexQuery {
                    title: (*title).into(),
                    query,
                })
                .collect(),
            call_id: None,
        })
        .await?;
    assert_eq!(response.result.len(), titles.len());
    assert!(response.result.iter().all(|group| group.entries.is_empty()));
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_rejected_sign_in_fails_requests_with_its_record_and_keeps_the_binding() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;

    // 401 on the request and on the storefront read that follows it: the sign-in is over. A
    // song the library does not list is read from Apple, as nothing kept it yet.
    apple.fake.reject_everything.store(true, Ordering::SeqCst);
    let unlisted = apple.binding.intern(AppleIdKind::LibrarySong, "i.s9")?;
    let error = apple
        .run(&FetchParsedMediaFileRequest {
            call_id: None,
            id: unlisted,
        })
        .await
        .err()
        .context("rejected sign-in")?;
    assert_eq!(
        request_failure_cause(&error),
        Some(RequestFailureCause::ServiceSignInRequired {
            record_id: apple.record.0.to_string(),
        })
    );
    assert_eq!(
        classify(&error),
        crate::messages::RequestErrorKind::PermissionDenied
    );
    // The binding is not signed out: the app signs in again and goes on.
    assert!(apple.binding.is_signed_in());
    apple.fake.reject_everything.store(false, Ordering::SeqCst);
    assert_eq!(
        apple
            .run(&GetMediaFilesCountRequest { call_id: None })
            .await?
            .count,
        3
    );

    // A 401 on one read while the storefront read works is answered by the library's own
    // second try, so the request answers normally.
    apple.fake.reject_next_read.store(true, Ordering::SeqCst);
    let before = apple.fake.count("/v1/me/library/artists");
    // Artists are read afresh, unlike the songs kept by now.
    let artists = apple
        .run(&FetchCollectionGroupSummaryRequest {
            call_id: None,
            collection_type: CollectionType::Artist,
        })
        .await?;
    assert_eq!(artists.groups[0].count, 2);
    // The library's own second try after the 401: this pins that retry.
    assert_eq!(apple.fake.count("/v1/me/library/artists"), before + 2);

    // A read Apple refuses again while the storefront read still works is not a lost
    // sign-in: the request fails as an ordinary error and asks for no new sign-in.
    *apple.fake.respond_path.lock().unwrap() = Some((
        "/v1/catalog/it/songs".into(),
        401,
        r#"{"errors":[{"status":"401","code":"UNAUTHORIZED"}]}"#.into(),
    ));
    let error = catalog_song(&apple)
        .await
        .err()
        .context("refused request")?;
    assert_eq!(request_failure_cause(&error), None);
    assert_eq!(
        classify(&error),
        crate::messages::RequestErrorKind::Unavailable
    );
    assert!(apple.binding.is_signed_in());
    *apple.fake.respond_path.lock().unwrap() = None;
    assert_eq!(catalog_song(&apple).await?.file.title, "Catalog Tune");
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn covers_download_once_without_credentials_and_back_the_visualizer() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    apple
        .run(&GetMediaFilesCountRequest { call_id: None })
        .await?;

    let (summary, _) = apple.binding.item_metadata("i.s1", true).await?;
    assert_eq!(summary.title, "First Light");
    // The cover downloads in the background; asking again finds it once it is there.
    let path = wait_until("the cover is cached", || async {
        Ok(apple.binding.item_metadata("i.s1", true).await?.1)
    })
    .await?;
    assert_eq!(std::fs::read(&path)?, b"cover-bytes");
    assert_eq!(apple.fake.count("/art/Dawn_Album/600x600bb.jpg"), 1);
    assert!(
        apple
            .fake
            .seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(target, _)| target.starts_with("/art/"))
            .all(|(_, authed)| !authed),
        "a cover download carried Apple credentials"
    );

    let (playing, wall) = apple.binding.visualizer_covers("i.s1").await?;
    assert_eq!(playing.as_deref(), Some(path.as_str()));
    // The other album's cover joins the wall.
    assert_eq!(wall.len(), 1);
    assert_ne!(wall[0], path);
    // The playing cover came from the cache.
    assert_eq!(apple.fake.count("/art/Dawn_Album"), 1);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parsed_media_files_share_collection_ids_and_take_catalog_songs() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let ids_of = |collection_type| {
        let title = if collection_type == CollectionType::Album {
            "D"
        } else {
            "A"
        };
        let apple = &apple;
        async move {
            Ok::<_, anyhow::Error>(
                apple
                    .run(&FetchCollectionGroupsRequest {
                        call_id: None,
                        collection_type,
                        bake_cover_arts: false,
                        group_titles: vec![title.into()],
                    })
                    .await?
                    .groups
                    .remove(0)
                    .collections
                    .into_iter()
                    .map(|collection| (collection.name, collection.id))
                    .collect::<std::collections::HashMap<_, _>>(),
            )
        }
    };
    let albums = ids_of(CollectionType::Album).await?;
    let artists = ids_of(CollectionType::Artist).await?;
    let first = apple.binding.intern(AppleIdKind::LibrarySong, "i.s1")?;
    let parsed = apple
        .run(&FetchParsedMediaFileRequest {
            call_id: None,
            id: first,
        })
        .await?;
    assert_eq!(parsed.album.id, albums["Dawn Album"]);
    assert_eq!(parsed.artists[0].id, artists["Artist One"]);

    let catalog = apple.binding.intern(AppleIdKind::CatalogSong, "777")?;
    let parsed = apple
        .run(&FetchParsedMediaFileRequest {
            call_id: None,
            id: catalog,
        })
        .await?;
    assert_eq!(parsed.file.track, "catalog:777");
    assert_eq!(parsed.file.title, "Catalog Tune");
    assert_eq!(parsed.album.name, "Catalog Album");
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn paging_past_the_end_is_empty_and_a_negative_position_is_an_input_error() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let past = apple
        .run(&FetchMediaFilesRequest {
            call_id: None,
            cursor: 100,
            page_size: 10,
            bake_cover_arts: false,
        })
        .await?;
    assert!(past.media_files.is_empty());
    let mut page = mix(
        "lib::album",
        apple.binding.intern(AppleIdKind::Album, "l.alb1")?,
    );
    page.cursor = 100;
    assert!(apple.run(&page).await?.files.is_empty());

    let invalid = crate::messages::RequestErrorKind::InvalidInput;
    let negative = apple
        .run(&FetchMediaFilesRequest {
            call_id: None,
            cursor: -1,
            page_size: 10,
            bake_cover_arts: false,
        })
        .await
        .err()
        .context("negative cursor")?;
    assert_eq!(classify(&negative), invalid);
    page.cursor = -1;
    let negative = apple
        .run(&page)
        .await
        .err()
        .context("negative mix cursor")?;
    assert_eq!(classify(&negative), invalid);
    page.cursor = 0;
    page.page_size = -1;
    let negative = apple.run(&page).await.err().context("negative page size")?;
    assert_eq!(classify(&negative), invalid);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_empty_library_has_no_songs_and_no_groups() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_listing(dir.path(), true).await?;
    assert_eq!(
        apple
            .run(&GetMediaFilesCountRequest { call_id: None })
            .await?
            .count,
        0
    );
    for collection_type in [CollectionType::Album, CollectionType::Artist] {
        let summary = apple
            .run(&FetchCollectionGroupSummaryRequest {
                call_id: None,
                collection_type,
            })
            .await?;
        assert!(summary.groups.is_empty(), "{collection_type:?}");
    }
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_answer_apple_rejects_carries_its_status_and_code_but_no_token() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    *apple.fake.respond_path.lock().unwrap() = Some((
        "/v1/catalog/it/songs".into(),
        400,
        r#"{"errors":[{"status":"400","code":"PARAMETER_ERROR"}]}"#.into(),
    ));
    // A song of the catalog is read from the catalog.
    let error = catalog_song(&apple)
        .await
        .err()
        .context("rejected request")?;
    let text = format!("{error:#}");
    assert!(
        text.contains("status 400") && text.contains("PARAMETER_ERROR"),
        "{text}"
    );
    assert!(!text.contains(USER_TOKEN) && !text.contains(DEVELOPER_TOKEN));
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cached_summary_never_starts_the_read_the_song_list_needs() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    // A binding nothing has read yet knows no song, and does not go and ask.
    let setup = fake_services(&apple.fake.base, &TestClock::new(), &apple.lrclib);
    let unread = AppleMusicBinding::new(
        uuid::Uuid::new_v4(),
        |tokens| setup.apple_music_client(tokens),
        dir.path(),
        Arc::new(Quiet),
        Arc::new(SystemClock),
        setup.lyrics.clone(),
    );
    unread.sign_in(Tokens::new(DEVELOPER_TOKEN, USER_TOKEN).context("tokens")?);
    let before = apple.fake.total();
    assert!(unread.cached_summary("i.s1").is_none());
    assert_eq!(apple.fake.total(), before);

    // The opened library has read its song list, and answers from it.
    let summary = apple
        .binding
        .cached_summary("i.s2")
        .context("listed song")?;
    assert_eq!(
        (
            summary.title.as_str(),
            summary.album.as_str(),
            summary.artist.as_str()
        ),
        ("Second Wind", "Dawn Album", "Artist One")
    );
    assert_eq!(summary.track_number, 2);
    assert!((summary.duration - 200.0).abs() < f64::EPSILON);
    assert!(apple.binding.cached_summary("i.unlisted").is_none());
    assert!(apple.binding.cached_summary("not a key").is_none());
    assert_eq!(apple.fake.total(), before);
    apple.finish().await
}

/// Writes a Subsonic record that points at `endpoint`.
async fn write_subsonic_record(
    node: &Arc<NodeContext>,
    endpoint: String,
) -> Result<LibraryRecordId> {
    use database::credentials::{Credential, Secret};
    use database::library_registry::{
        CatalogPlacement, LibraryRecord, LibraryRegistry, ProtocolAdapter, SubsonicOpenMode,
    };

    let credential = node
        .credentials
        .put(Credential {
            username: "account".into(),
            secret: Secret::new(b"unused".to_vec()),
        })
        .await?;
    let record = LibraryRecord {
        record_id: LibraryRecordId(uuid::Uuid::new_v4()),
        alias: "Closed service".into(),
        library_id: None,
        protocol: ProtocolAdapter::SubsonicTranslate {
            endpoint,
            credential,
            mode: SubsonicOpenMode::PlayOnThisDevice,
        },
        fs: Vec::new(),
        catalog: CatalogPlacement::None,
        background_sync: false,
    };
    let record_id = record.record_id;
    LibraryRegistry::update(Path::new(&*node.config_path), |registry| {
        registry.libraries.push(record);
        Ok(())
    })
    .await?;
    Ok(record_id)
}

/// A library that answers lyrics and likes itself, standing for a Subsonic one: like a
/// Subsonic library's, its answers are only for its own items. `slots` are how it runs requests
/// as the open library.
struct LyricService {
    library_id: uuid::Uuid,
    slots: RequestSlots,
    liked: Mutex<std::collections::HashSet<String>>,
}

impl LyricService {
    fn with_slots(library_id: uuid::Uuid, slots: RequestSlots) -> Self {
        Self {
            library_id,
            slots,
            liked: Mutex::default(),
        }
    }

    /// As the stand-in was before it took a Subsonic library's slots: lyrics and colors are
    /// left to the library of each item named.
    fn routing_by_item(library_id: uuid::Uuid) -> Self {
        let mut slots = RequestSlots::unimplemented();
        slots.get_lyric_by_track_id_request = Slot::Native;
        slots.get_primary_color_by_track_id_request = Slot::Native;
        Self::with_slots(library_id, slots)
    }

    /// The track `item` names, refused as a Subsonic library refuses an item of another one.
    fn own_track(&self, item: &Option<PlayingItemRef>) -> Result<String> {
        match item {
            Some(PlayingItemRef::LibraryTrack { library_id, track })
                if *library_id == self.library_id.to_string() =>
            {
                Ok(track.clone())
            }
            _ => Err(ExpectedRequestError::invalid_input("Item belongs to another library").into()),
        }
    }
}

impl crate::macros::RequestTranslator for LyricService {
    fn translate_get_lyric_by_track_id_request<'a>(
        &'a self,
        request: &'a GetLyricByTrackIdRequest,
    ) -> crate::rpc::RequestTranslationFuture<'a, GetLyricByTrackIdResponse> {
        Box::pin(async move {
            self.own_track(&request.item)?;
            Ok(Some(GetLyricByTrackIdResponse {
                item: request.item.clone().context("lyric item")?,
                lines: vec![LyricContentLine {
                    start_time: 0,
                    end_time: 1,
                    sections: Vec::new(),
                }],
                call_id: None,
            }))
        })
    }

    fn translate_set_liked_request<'a>(
        &'a self,
        request: &'a SetLikedRequest,
    ) -> crate::rpc::RequestTranslationFuture<'a, crate::messages::SetLikedResponse> {
        Box::pin(async move {
            let track = self.own_track(&request.item)?;
            let mut liked = self.liked.lock().unwrap();
            if request.liked {
                liked.insert(track);
            } else {
                liked.remove(&track);
            }
            Ok(Some(crate::messages::SetLikedResponse {
                item: request.item.clone().context("liked item")?,
                liked: request.liked,
                call_id: None,
            }))
        })
    }

    fn translate_get_liked_request<'a>(
        &'a self,
        request: &'a crate::messages::GetLikedRequest,
    ) -> crate::rpc::RequestTranslationFuture<'a, crate::messages::GetLikedResponse> {
        Box::pin(async move {
            let track = self.own_track(&request.item)?;
            Ok(Some(crate::messages::GetLikedResponse {
                item: request.item.clone().context("liked item")?,
                liked: self.liked.lock().unwrap().contains(&track),
                call_id: None,
            }))
        })
    }
}

impl ServiceBinding for LyricService {
    fn library_id(&self) -> uuid::Uuid {
        self.library_id
    }

    fn slots(&self) -> RequestSlots {
        self.slots
    }

    fn service(&self) -> ServiceKind {
        ServiceKind::Subsonic
    }

    fn item_metadata<'a>(
        &'a self,
        _track: &'a str,
        _with_cover: bool,
    ) -> crate::service::ServiceFuture<
        'a,
        (
            database::playing_item::PlayingItemMetadataSummary,
            Option<String>,
        ),
    > {
        Box::pin(async { anyhow::bail!("no metadata") })
    }

    fn visualizer_covers<'a>(
        &'a self,
        _track: &'a str,
    ) -> crate::service::ServiceFuture<'a, (Option<String>, Vec<String>)> {
        Box::pin(async { anyhow::bail!("no covers") })
    }

    fn playback_source<'a>(
        &'a self,
        _track: &'a str,
    ) -> crate::service::ServiceFuture<'a, playback::player::ServiceTarget> {
        Box::pin(async { anyhow::bail!("no playback") })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lyrics_and_colors_go_to_the_library_each_item_belongs_to() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let local_library = apple
        .fixture
        .global_params
        .library_id
        .context("fixture library id")?;
    // Another streaming library is the one open, so the Apple item is a foreign one.
    let other_record = write_subsonic_record(&apple.node, "http://127.0.0.1:9".into()).await?;
    let other = other_record.0;
    apple
        .node
        .open_service_catalog(
            Some(other_record),
            Arc::new(LyricService::routing_by_item(other)),
        )
        .await?;
    let item = |library_id: uuid::Uuid, track: &str| PlayingItemRef::LibraryTrack {
        library_id: library_id.to_string(),
        track: track.into(),
    };
    let apple_item = item(apple.record.0, "i.s1");
    let other_item = item(other, "t1");
    let local_item = item(local_library, &uuid::Uuid::new_v4().to_string());
    // The UI's own params of an open streaming library hold no local library.
    let ui = apple
        .node
        .current_ui()
        .await
        .context("UI params of the open library")?;
    assert!(ui.library.is_none());
    let requests_before = apple.fake.total();

    for params in [&ui, &apple.fixture.global_params] {
        let lyric = |item: &PlayingItemRef| {
            let request = GetLyricByTrackIdRequest {
                call_id: None,
                item: Some(item.clone()),
            };
            async move { dispatch_gui_request(params, &request).await }
        };
        // An Apple item's library declares no lyrics for its items, which it refuses.
        let refused = lyric(&apple_item).await.err().context("Apple lyrics")?;
        assert!(unsupported(&refused), "{refused:#}");
        // Another streaming library's item gets that library's own answer.
        let answered = lyric(&other_item)
            .await?
            .context("other library's lyrics")?;
        assert_eq!(answered.lines.len(), 1);
        // A local item reaches its own library, which has nothing for it.
        let local = lyric(&local_item).await?.context("local lyrics")?;
        assert!(local.lines.is_empty());

        let color = |item: &PlayingItemRef| {
            let request = GetPrimaryColorByTrackIdRequest {
                call_id: None,
                item: Some(item.clone()),
            };
            async move { dispatch_gui_request(params, &request).await }
        };
        // Without a local library open, a misrouted Apple item has no library to read from
        // and fails; routed to its binding it answers no color.
        let apple_color = color(&apple_item).await?.context("Apple color")?;
        assert_eq!(apple_color.primary_color, None);
        assert!(color(&local_item).await?.is_some());
    }
    // With no item the request still needs an open local library, as it always did.
    let none = GetLyricByTrackIdRequest {
        call_id: None,
        item: None,
    };
    assert!(dispatch_gui_request(&ui, &none).await.is_err());
    assert!(
        dispatch_gui_request(&apple.fixture.global_params, &none)
            .await?
            .is_none()
    );
    assert_eq!(apple.fake.total(), requests_before);
    apple.finish().await
}

/// A Subsonic library's slots, as a real binding gets them from a server that has no jukebox.
fn subsonic_slots() -> RequestSlots {
    RequestSlots::subsonic(&subsonic::SubsonicProbe {
        api_version: "1.16.1".into(),
        open_subsonic: false,
        server_type: None,
        server_version: None,
        extensions: Vec::new(),
        roles: subsonic::SubsonicRoles::default(),
        roles_known: true,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_open_subsonic_library_leaves_the_lyrics_of_other_libraries_items_to_them() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let local = local_track(&apple.fixture, "beside-subsonic.ogg").await?;
    let record = write_subsonic_record(&apple.node, "http://127.0.0.1:9".into()).await?;
    apple
        .node
        .open_service_catalog(
            Some(record),
            Arc::new(LyricService::with_slots(record.0, subsonic_slots())),
        )
        .await?;
    let ui = apple
        .node
        .current_ui()
        .await
        .context("UI params of the open library")?;
    let lyric = |item: PlayingItemRef| {
        let request = GetLyricByTrackIdRequest {
            call_id: None,
            item: Some(item),
        };
        let ui = &ui;
        async move { dispatch_gui_request(ui, &request).await }
    };

    // A local track's lyrics come from its own library, which has none for it.
    let local_lyrics = lyric(local).await?.context("local lyrics")?;
    assert!(local_lyrics.lines.is_empty());
    // An Apple Music track's library declares no lyrics, whichever library is open.
    let refused = lyric(song_ref(&apple, "i.s1"))
        .await
        .err()
        .context("Apple lyrics")?;
    assert!(unsupported(&refused), "{refused:#}");
    // The Subsonic library still answers for its own tracks.
    let own = lyric(PlayingItemRef::LibraryTrack {
        library_id: record.0.to_string(),
        track: "s1".into(),
    })
    .await?
    .context("Subsonic lyrics")?;
    assert_eq!(own.lines.len(), 1);
    apple.finish().await
}

/// Asks each request routed by item about `item` through `params`, and checks it fails as
/// unsupported exactly where `declared` hides it. Any other failure must be one the UI takes
/// as such, as a library out of reach, never an internal one.
async fn asking_matches(
    params: &crate::utils::GlobalParams,
    item: &PlayingItemRef,
    declared: &crate::messages::PlayingItemCapabilities,
) -> Result<()> {
    use crate::messages::{GetLikedRequest, RequestErrorKind};

    let named = Some(item.clone());
    let asked = [
        (
            "SetLikedRequest",
            declared.set_liked_request,
            dispatch_gui_request(
                params,
                &SetLikedRequest {
                    item: named.clone(),
                    liked: true,
                    call_id: None,
                },
            )
            .await
            .err(),
        ),
        (
            "GetLikedRequest",
            declared.get_liked_request,
            dispatch_gui_request(
                params,
                &GetLikedRequest {
                    item: named.clone(),
                    call_id: None,
                },
            )
            .await
            .err(),
        ),
        (
            "GetLyricByTrackIdRequest",
            declared.get_lyric_by_track_id_request,
            dispatch_gui_request(
                params,
                &GetLyricByTrackIdRequest {
                    item: named.clone(),
                    call_id: None,
                },
            )
            .await
            .err(),
        ),
        (
            "GetPrimaryColorByTrackIdRequest",
            declared.get_primary_color_by_track_id_request,
            dispatch_gui_request(
                params,
                &GetPrimaryColorByTrackIdRequest {
                    item: named,
                    call_id: None,
                },
            )
            .await
            .err(),
        ),
    ];
    for (request, capability, error) in asked {
        let refused = error.as_ref().is_some_and(unsupported);
        assert_eq!(
            capability == RequestCapability::Hidden,
            refused,
            "{request} for {item:?}: {capability:?}, {error:?}"
        );
        if let Some(error) = &error {
            assert_ne!(
                classify(error),
                RequestErrorKind::Internal,
                "{request} for {item:?}: {error:#}"
            );
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_an_item_supports_is_what_asking_about_it_does() -> Result<()> {
    use crate::macros::item_capability_snapshot;
    use crate::request_owner::Implementation;
    use crate::service::item_slots;

    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let local = local_track(&apple.fixture, "asked-about.ogg").await?;
    let song = song_ref(&apple, "i.s1");
    let subsonic_record = write_subsonic_record(&apple.node, "http://127.0.0.1:9".into()).await?;
    let subsonic: Arc<dyn ServiceBinding> = Arc::new(LyricService::with_slots(
        subsonic_record.0,
        subsonic_slots(),
    ));
    let subsonic_track = PlayingItemRef::LibraryTrack {
        library_id: subsonic_record.0.to_string(),
        track: "s1".into(),
    };
    let apple_binding: Arc<dyn ServiceBinding> = apple.binding.clone();

    // Each binding is of the service its record names, so what a signed-in binding declares
    // for its items is what the record declares.
    let registry = LibraryRegistry::load(Path::new(&*apple.node.config_path))?;
    for (record, binding) in [(apple.record, &apple_binding), (subsonic_record, &subsonic)] {
        let protocol = &registry
            .record(record)
            .map_err(|_| anyhow::anyhow!("the record of {record:?}"))?
            .protocol;
        let service = protocol.service().context("a service record")?;
        assert_eq!(binding.service(), service);
        assert_eq!(
            item_capability_snapshot(None, &Implementation::Service(binding.clone()).item_slots()),
            item_capability_snapshot(None, &item_slots(service, binding.online_lyrics()))
        );
    }
    let declared = |item: &PlayingItemRef, implementation: Implementation| {
        item_capability_snapshot(Some(item.clone()), &implementation.item_slots())
    };
    let items = [
        declared(&song, Implementation::Service(apple_binding.clone())),
        declared(&local, Implementation::Local),
        declared(&subsonic_track, Implementation::Service(subsonic.clone())),
    ];
    let ask_all = |params: &crate::utils::GlobalParams| {
        let items = items.clone();
        let params = params.clone();
        async move {
            for told in &items {
                let item = told.item.as_ref().context("a declared item")?;
                asking_matches(&params, item, told).await?;
            }
            anyhow::Ok(())
        }
    };

    // With the Apple Music library open, asked through it and through the local library.
    let apple_ui = Arc::clone(&apple.ui);
    ask_all(&apple_ui).await?;
    ask_all(&apple.fixture.global_params).await?;

    // With the Subsonic library open.
    apple
        .node
        .open_service_catalog(Some(subsonic_record), subsonic.clone())
        .await?;
    let subsonic_ui = apple
        .node
        .current_ui()
        .await
        .context("UI params of the Subsonic library")?;
    ask_all(&subsonic_ui).await?;

    // With the local library open.
    open_own_library(&apple).await?;
    ask_all(&apple.fixture.global_params).await?;
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn task_events_of_a_device_only_library_are_withheld_from_other_devices() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let policy = &apple.node.access_policy;
    let task = |record_id: String| {
        rinf::serialize(&LibraryTaskStateChanged {
            state: LibraryTaskState::AnalysisRunning {
                record_id,
                run_id: "run".into(),
                progress: 1,
                total: 2,
            },
        })
    };
    let local_record = uuid::Uuid::new_v4().to_string();
    // The Apple library is open and kept on this device; its own task event stays here.
    assert!(withheld_library_event(
        policy,
        "LibraryTaskStateChanged",
        &task(apple.record.0.to_string())?
    ));
    // A local library's event goes on telling other devices while Apple is open.
    assert!(!withheld_library_event(
        policy,
        "LibraryTaskStateChanged",
        &task(local_record)?
    ));
    // Other events name no library and are not decided here, and a task event nobody can
    // read is not let through.
    let content = rinf::serialize(&LibraryContentChanged {
        path: String::new(),
    })?;
    assert!(!withheld_library_event(
        policy,
        "LibraryContentChanged",
        &content
    ));
    assert!(!withheld_library_event(policy, "PlaybackStatus", &[]));
    assert!(withheld_library_event(
        policy,
        "LibraryTaskStateChanged",
        &[1, 2, 3]
    ));
    apple.finish().await
}

/// What a device's broadcaster was told, to see what reached it over the network.
#[derive(Default)]
struct Heard(Mutex<Vec<(String, Vec<u8>)>>);

impl crate::utils::Broadcaster for Heard {
    fn broadcast(&self, message: &dyn crate::utils::RinfRustSignal) {
        self.0
            .lock()
            .unwrap()
            .push((message.name(), message.encode_to_vec().unwrap()));
    }
}

impl Heard {
    fn task_records(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == "LibraryTaskStateChanged")
            .filter_map(|(_, payload)| rinf::deserialize::<LibraryTaskStateChanged>(payload).ok())
            .map(|changed| changed.state.record_id().to_owned())
            .collect()
    }

    fn count(&self, name: &str) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(heard, _)| heard == name)
            .count()
    }

    /// Every message named `name` that decodes as `T`, with its place among all heard.
    fn placed<T: serde::de::DeserializeOwned>(&self, name: &str) -> Vec<(usize, T)> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .enumerate()
            .filter(|(_, (heard, _))| heard == name)
            .filter_map(|(place, (_, payload))| {
                rinf::deserialize::<T>(payload)
                    .ok()
                    .map(|message| (place, message))
            })
            .collect()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_outgoing_loop_keeps_a_device_only_librarys_task_events_off_the_wire() -> Result<()> {
    let host_dir = tempfile::tempdir()?;
    let controller_dir = tempfile::tempdir()?;
    let host = open_apple(host_dir.path()).await?;
    let heard = Arc::new(Heard::default());
    let controller = super::fixture_with_broadcaster(
        controller_dir.path(),
        uuid::Uuid::new_v4(),
        std::time::Duration::from_secs(300),
        heard.clone(),
    )
    .await?;
    // The host's own library is open while the devices pair, as in a real session.
    let host_library = host
        .fixture
        .global_params
        .library_id
        .context("host library id")?;
    host.node.set_open_local_catalog(None, host_library).await?;
    let address = super::paired_full(&host.fixture, &controller).await?;
    let fingerprint = super::info(&host.fixture).await?.fingerprint;
    controller
        .global_params
        .cert_validator
        .read()
        .await
        .replace_hosts_for_fingerprint(&fingerprint, vec![format!("127.0.0.1:{}", address.port())])
        .await?;
    super::open_remote_control_record(controller.global_params.node.clone(), &fingerprint).await?;
    host.node
        .open_service_catalog(Some(host.record), host.binding.clone())
        .await?;

    let task = |record_id: String| LibraryTaskStateChanged {
        state: LibraryTaskState::AnalysisRunning {
            record_id,
            run_id: "run".into(),
            progress: 1,
            total: 2,
        },
    };
    let local_record = uuid::Uuid::new_v4().to_string();
    let broadcaster = &host.fixture.global_params.broadcaster;
    broadcaster.broadcast(&task(host.record.0.to_string()));
    broadcaster.broadcast(&task(local_record.clone()));
    // A content change the test alone sends marks the end; the host may tell of others.
    const MARKER: &str = "marker";
    broadcaster.broadcast(&LibraryContentChanged {
        path: MARKER.into(),
    });
    // Events reach the controller in the order they were sent, so once the last has arrived
    // the first would have.
    wait_until("the content change to arrive", || async {
        let changes = heard.placed::<LibraryContentChanged>("LibraryContentChanged");
        Ok(changes
            .iter()
            .any(|(_, changed)| changed.path == MARKER)
            .then_some(()))
    })
    .await?;
    assert_eq!(heard.task_records(), vec![local_record]);
    host.server.abort();
    host.node.shutdown().await?;
    controller.global_params.node.shutdown().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn removing_the_record_stops_covers_that_are_still_downloading() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    *apple.fake.delay_path.lock().unwrap() =
        Some(("/art/".into(), std::time::Duration::from_secs(2)));
    apple
        .binding
        .cover_path(Some(&format!(
            "{}/art/Dawn_Album/600x600bb.jpg",
            apple.fake.base
        )))
        .await;
    wait_until("the cover request to arrive", || async {
        Ok((apple.fake.count("/art/") > 0).then_some(()))
    })
    .await?;
    let config = std::path::PathBuf::from(&*apple.node.config_path);
    let folder = cover_cache_root(&config, apple.record.0);
    apple.node.services.retire(apple.record).await;
    delete_service_library_folder(&config, apple.record.0, false).await?;
    // Apple's answer comes after the removal; it must not bring the folder back.
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    assert!(
        !folder.exists(),
        "a cover made its folder again after the removal"
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_read_begun_before_a_sign_out_is_not_kept() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    *apple.fake.delay_path.lock().unwrap() = Some((
        "/v1/me/library/artists".into(),
        std::time::Duration::from_millis(600),
    ));
    let read = apple.binding.artists();
    let switch = async {
        wait_until("the artists request to arrive", || async {
            Ok((apple.fake.count("/v1/me/library/artists") > 0).then_some(()))
        })
        .await?;
        apple.binding.sign_out();
        apple
            .binding
            .sign_in(Tokens::new(DEVELOPER_TOKEN, APPLE_OTHER_TOKEN).context("tokens")?);
        Ok::<_, anyhow::Error>(())
    };
    let (read, switch) = tokio::join!(read, switch);
    // The read still answers whoever waited for it.
    read?;
    switch?;
    // What that read returned was not kept, so the next one asks Apple again.
    *apple.fake.delay_path.lock().unwrap() = None;
    let before = apple.fake.count("/v1/me/library/artists");
    apple.binding.artists().await?;
    assert_eq!(apple.fake.count("/v1/me/library/artists"), before + 1);
    // Read under the new account, it is kept.
    apple.binding.artists().await?;
    assert_eq!(apple.fake.count("/v1/me/library/artists"), before + 1);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_song_list_read_begun_before_a_sign_out_is_not_known_after_it() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let release = Arc::new(tokio::sync::Notify::new());
    *apple.fake.hold_once.lock().unwrap() =
        Some(("/v1/me/library/songs".into(), release.clone()));
    let reads = apple.fake.count("/v1/me/library/songs");
    let read = apple.binding.current_songs();
    let switch = async {
        fake_asked(&apple, "/v1/me/library/songs", reads + 1).await?;
        apple.binding.sign_out();
        release.notify_one();
        Ok::<_, anyhow::Error>(())
    };
    let (read, switch) = tokio::join!(read, switch);
    switch?;
    // The caller that waited still gets what was read, and the binding knows none of it.
    assert_eq!(read?.len(), 3);
    assert!(apple.binding.known_songs().is_none());
    assert!(apple.binding.known_song("i.s1").is_none());
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_old_list_answers_at_once_and_the_ui_hears_once_apple_changed_it() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (apple, heard) = open_apple_heard(dir.path()).await?;
    // The artists are read when the artist list is opened; "Late Night" is not there yet.
    let listed = async || -> Result<i32> {
        Ok(apple
            .run(&FetchCollectionGroupSummaryRequest {
                call_id: None,
                collection_type: CollectionType::Artist,
            })
            .await?
            .groups
            .iter()
            .map(|group| group.count)
            .sum())
    };
    assert_eq!(listed().await?, 2);

    // Apple lists another artist, the kept list grows old, and Apple's next answer is held.
    apple
        .fake
        .more_artists
        .lock()
        .unwrap()
        .push(AppleFake::artist("r.art3", "Late Night"));
    apple
        .clock
        .advance(crate::apple_music_binding::SHORT_LIST_AGE);
    let release = Arc::new(tokio::sync::Notify::new());
    *apple.fake.hold_once.lock().unwrap() =
        Some(("/v1/me/library/artists".into(), release.clone()));
    let reads = apple.fake.count("/v1/me/library/artists");
    let told = told_settled(&heard).await;
    // The page answers at once with the kept list, and Apple is asked again behind it, once.
    assert_eq!(listed().await?, 2);
    fake_asked(&apple, "/v1/me/library/artists", reads + 1).await?;
    assert_eq!(listed().await?, 2);
    assert_eq!(heard.count(CONTENT_CHANGED), told);

    // Apple's answer differs from what the page showed: the UI is told, and the list grows.
    release.notify_one();
    wait_until("the UI to hear the artists changed", || async {
        Ok((heard.count(CONTENT_CHANGED) > told).then_some(()))
    })
    .await?;
    assert_eq!(listed().await?, 3);
    assert_eq!(apple.fake.count("/v1/me/library/artists"), reads + 1);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_binding_made_afresh_answers_the_artist_list_from_disk_before_its_sign_in() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let artists = apple
        .run(&FetchCollectionGroupSummaryRequest {
            call_id: None,
            collection_type: CollectionType::Artist,
        })
        .await?;
    assert_eq!(
        artists.groups.iter().map(|group| group.count).sum::<i32>(),
        2
    );
    let config = std::path::PathBuf::from(&*apple.node.config_path);
    let lists = lists_folder(&config, apple.record.0);
    wait_until("the artists and songs to be kept on disk", || async {
        let kept = lists.join("artists.json").exists() && lists.join("songs.json").exists();
        Ok(kept.then_some(()))
    })
    .await?;

    // As after a restart, before the platform signs the library in again.
    let setup = fake_services(&apple.fake.base, &TestClock::new(), &apple.lrclib);
    let fresh = AppleMusicBinding::new(
        apple.record.0,
        |tokens| setup.apple_music_client(tokens),
        &config,
        Arc::new(Heard::default()),
        Arc::new(SystemClock),
        setup.lyrics.clone(),
    );
    assert!(!fresh.is_signed_in());
    let asked = apple.fake.total();
    let named: Vec<String> = fresh
        .named_artists()
        .await?
        .iter()
        .map(|artist| artist.name.clone())
        .collect();
    assert_eq!(named, ["Artist One", "Artist Two"]);
    assert_eq!(fresh.songs().await?.len(), 3);
    assert_eq!(apple.fake.total(), asked);

    // Signing out deletes them, and nothing answers until a sign-in.
    fresh.sign_out();
    assert!(!lists.exists());
    let error = fresh.artists().await.err().context("a signed-out read")?;
    assert_eq!(
        request_failure_cause(&error),
        Some(RequestFailureCause::ServiceSignInRequired {
            record_id: apple.record.0.to_string(),
        })
    );
    apple.finish().await
}

/// An Apple Music library as the next start of the app finds it, before the platform signs it
/// in: the node starts again on the configuration a signed-in run left, with the library's
/// lists kept on disk.
struct Restarted {
    node: Arc<NodeContext>,
    heard: Arc<Heard>,
    fake: Arc<AppleFake>,
    record: LibraryRecordId,
    server: tokio::task::JoinHandle<()>,
    /// The run before, whose files the new node takes over.
    _before: Fixture,
}

/// Has `apple` keep the songs, the albums and the artists on disk, as the pages that show them
/// read them, and starts the node again on its configuration.
async fn restart_apple(apple: OpenApple) -> Result<Restarted> {
    for collection_type in [CollectionType::Album, CollectionType::Artist] {
        apple
            .run(&FetchCollectionGroupSummaryRequest {
                call_id: None,
                collection_type,
            })
            .await?;
    }
    let lists = lists_folder(Path::new(&*apple.node.config_path), apple.record.0);
    wait_until(
        "the songs, albums and artists to be kept on disk",
        || async {
            let kept = ["songs", "albums", "artists"]
                .iter()
                .all(|name| lists.join(format!("{name}.json")).exists());
            Ok(kept.then_some(()))
        },
    )
    .await?;
    apple.node.shutdown().await?;
    let heard = Arc::new(Heard::default());
    let config = apple.fixture.global_params.config_path.clone();
    let node = node_on(&config, heard.clone(), &apple.fake.base).await?;
    let OpenApple {
        fixture,
        fake,
        record,
        server,
        ..
    } = apple;
    Ok(Restarted {
        node,
        heard,
        fake,
        record,
        server,
        _before: fixture,
    })
}

impl Restarted {
    /// Opens the library as the UI does, before any sign-in, and answers the UI's params and
    /// the binding the open made.
    async fn open(&self) -> Result<(Arc<crate::utils::GlobalParams>, Arc<AppleMusicBinding>)> {
        assert!(
            self.node
                .services
                .apple_music_binding(self.record)
                .is_none()
        );
        let ui =
            open_registered_library_record(self.node.clone(), self.record.0.to_string()).await?;
        let binding = self
            .node
            .services
            .apple_music_binding(self.record)
            .context("the binding the open made")?;
        Ok((ui, binding))
    }

    /// Signs the library in as the platform does.
    async fn sign_in(&self, user_token: &str) -> Result<()> {
        use crate::messages::{ServiceSignIn, SignInServiceLibraryRequest};
        use crate::utils::sign_in_service_library_on_node;

        sign_in_service_library_on_node(
            &SignInServiceLibraryRequest {
                record_id: self.record.0.to_string(),
                sign_in: ServiceSignIn::Platform {
                    user_token: user_token.into(),
                    developer_token: Some(DEVELOPER_TOKEN.into()),
                },
                call_id: None,
            },
            &self.node,
        )
        .await
    }

    async fn finish(self) -> Result<()> {
        self.server.abort();
        self.node.shutdown().await
    }
}

/// The songs the Tracks page lists, as id, track and whether each plays.
async fn listed_songs(
    ui: &crate::utils::GlobalParams,
) -> Result<Vec<(i32, String, AudioAvailability)>> {
    let page = dispatch_gui_request(
        ui,
        &FetchMediaFilesRequest {
            call_id: None,
            cursor: 0,
            page_size: 50,
            bake_cover_arts: false,
        },
    )
    .await?
    .context("the song page")?;
    Ok(page
        .media_files
        .into_iter()
        .map(|file| (file.id, file.track, file.audio_availability))
        .collect())
}

/// How many entries each group of the collection type has, summed.
async fn collection_count(
    ui: &crate::utils::GlobalParams,
    collection_type: CollectionType,
) -> Result<i32> {
    Ok(dispatch_gui_request(
        ui,
        &FetchCollectionGroupSummaryRequest {
            call_id: None,
            collection_type,
        },
    )
    .await?
    .context("the group summary")?
    .groups
    .iter()
    .map(|group| group.count)
    .sum())
}

/// The home of the albums, artists and tracks, as entry names per group.
async fn home_names(ui: &crate::utils::GlobalParams) -> Result<Vec<Vec<String>>> {
    let mode = LibraryHomeListMode::Name;
    let queries = [
        LibraryHomeQuery::Albums(mode),
        LibraryHomeQuery::Artists(mode),
        LibraryHomeQuery::Tracks(mode),
    ];
    let response = dispatch_gui_request(
        ui,
        &ComplexQueryRequest {
            queries: queries
                .into_iter()
                .map(|query| ComplexQuery {
                    title: format!("{query:?}"),
                    query,
                })
                .collect(),
            call_id: None,
        },
    )
    .await?
    .context("the home")?;
    Ok(response
        .result
        .iter()
        .map(|group| {
            group
                .entries
                .iter()
                .map(|entry| entry.name.clone())
                .collect()
        })
        .collect())
}

/// A read that needs Apple: a song the kept song list does not hold.
async fn unlisted_song(
    ui: &crate::utils::GlobalParams,
    binding: &AppleMusicBinding,
) -> Result<crate::messages::MediaFile> {
    let id = binding.intern(AppleIdKind::LibrarySong, "i.s9")?;
    Ok(
        dispatch_gui_request(ui, &FetchParsedMediaFileRequest { call_id: None, id })
            .await?
            .context("the unlisted song")?
            .file,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_apple_library_opens_offline_from_what_it_kept_and_waits_for_its_sign_in() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let home_signed_in = home_names(&apple.fixture.global_params).await?;
    let restarted = restart_apple(apple).await?;
    // The device starts offline, with one song downloaded.
    restarted.node.services.reach_changed(
        crate::apple_music_binding::APPLE_MUSIC_HOST,
        Reach::offline(HashSet::from(["i.s2".to_owned()])),
    );
    let asked = restarted.fake.total();
    let (ui, binding) = restarted.open().await?;
    assert!(!binding.is_signed_in());

    // The pages answer from the lists kept on disk, the songs that cannot play offline greyed
    // out, and the browsing requests stay available.
    let songs = listed_songs(&ui).await?;
    assert_eq!(
        songs
            .iter()
            .map(|(_, track, playable)| (track.as_str(), *playable))
            .collect::<Vec<_>>(),
        [
            ("i.s1", AudioAvailability::Unavailable),
            ("i.s2", AudioAvailability::RemoteOnly),
            ("i.s3", AudioAvailability::Unavailable),
        ]
    );
    assert_eq!(
        dispatch_gui_request(&ui, &GetMediaFilesCountRequest { call_id: None })
            .await?
            .context("the song count")?
            .count,
        3
    );
    assert_eq!(collection_count(&ui, CollectionType::Album).await?, 2);
    assert_eq!(collection_count(&ui, CollectionType::Artist).await?, 2);
    // The home shows what it showed signed in.
    assert_eq!(home_names(&ui).await?, home_signed_in);
    assert_eq!(home_signed_in[0], ["Dawn Album", "Night Album"]);
    let capabilities = capability_snapshot(&restarted.node)?;
    for capability in [
        capabilities.fetch_media_files_request,
        capabilities.complex_query_request,
        capabilities.fetch_collection_group_summary_request,
    ] {
        assert_eq!(capability, RequestCapability::Available);
    }

    // A song plays on the host by its key, and is named from the kept list.
    match binding.playback_source("i.s2").await? {
        playback::player::ServiceTarget::Hosted(hosted) => assert_eq!(hosted.key, "i.s2"),
        _ => anyhow::bail!("an Apple Music song plays on the host"),
    }
    let (summary, _) = binding.item_metadata("i.s2", false).await?;
    assert_eq!(summary.title, "Second Wind");
    assert_eq!(restarted.fake.total(), asked);

    // The Tracks page resolves the song it shows as playable from the same kept list.
    let tracks_page = ResolveMixQueryRequest {
        queries: vec![MixQuery {
            operator: "lib::all".into(),
            parameter: "true".into(),
        }],
        hint_position: 0,
        initial_playback_item: Some(PlayingItemRef::LibraryTrack {
            library_id: restarted.record.0.to_string(),
            track: "i.s2".into(),
        }),
        instantly_play: true,
        fallback_playing_items: Vec::new(),
        excluded: ExcludedItems::default(),
        call_id: None,
    };
    let resolved = dispatch_gui_request(&ui, &tracks_page)
        .await?
        .context("the Tracks page offline")?;
    assert_eq!(resolved.start_index, Some(1));
    assert_eq!(resolved.playing_items.len(), 3);
    assert_eq!(restarted.fake.total(), asked);

    // A read that needs Apple fails as every request does offline.
    let error = unlisted_song(&ui, &binding)
        .await
        .err()
        .context("a read offline")?;
    assert_eq!(
        classify(&error),
        crate::messages::RequestErrorKind::Unavailable,
        "{error:#}"
    );
    assert_eq!(request_failure_cause(&error), None);
    // Online, it waits for the sign-in, which the UI takes up quietly.
    restarted.node.services.reach_changed(
        crate::apple_music_binding::APPLE_MUSIC_HOST,
        Reach::default(),
    );
    let error = unlisted_song(&ui, &binding)
        .await
        .err()
        .context("a read online before the sign-in")?;
    assert_eq!(
        request_failure_cause(&error),
        Some(RequestFailureCause::ServiceSignInRequired {
            record_id: restarted.record.0.to_string(),
        })
    );
    assert!(
        listed_songs(&ui)
            .await?
            .iter()
            .all(|(_, _, playable)| *playable == AudioAvailability::RemoteOnly)
    );
    assert_eq!(home_names(&ui).await?, home_signed_in);
    assert_eq!(restarted.fake.total(), asked);
    restarted.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sign_in_after_an_open_from_what_was_kept_refreshes_the_same_binding_and_ids()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let restarted = restart_apple(open_apple(dir.path()).await?).await?;
    let (ui, binding) = restarted.open().await?;
    let songs = listed_songs(&ui).await?;
    assert_eq!(collection_count(&ui, CollectionType::Artist).await?, 2);
    // The home shows what was kept.
    let home = home_names(&ui).await?;
    assert_eq!(home[1], ["Artist One", "Artist Two"]);
    assert_eq!(home[2].len(), 3);

    // Apple lists another artist by now, which the sign-in reads behind the kept list.
    restarted
        .fake
        .more_artists
        .lock()
        .unwrap()
        .push(AppleFake::artist("r.art3", "Late Night"));
    let (told, reads) = (
        told_settled(&restarted.heard).await,
        restarted.fake.count("/v1/me/library/songs"),
    );
    restarted.sign_in(USER_TOKEN).await?;
    let signed_in = restarted
        .node
        .services
        .apple_music_binding(restarted.record)
        .context("the signed-in binding")?;
    assert!(Arc::ptr_eq(&signed_in, &binding));
    assert!(binding.is_signed_in());
    wait_until("the UI to hear what Apple lists now", || async {
        Ok((restarted.heard.count(CONTENT_CHANGED) > told).then_some(()))
    })
    .await?;
    wait_until("the artist Apple added to be listed", || async {
        Ok((collection_count(&ui, CollectionType::Artist).await? == 3).then_some(()))
    })
    .await?;
    // The kept song list served before the sign-in is read again, once, and the songs keep
    // the ids the pages hold.
    fake_asked_at_least(&restarted, "/v1/me/library/songs", reads + 1).await?;
    assert_eq!(listed_songs(&ui).await?, songs);
    restarted.finish().await
}

/// Waits until Apple was asked for `prefix` at least `times` times.
async fn fake_asked_at_least(restarted: &Restarted, prefix: &str, times: usize) -> Result<()> {
    wait_until("Apple to be asked again", || async {
        Ok((restarted.fake.count(prefix) >= times).then_some(()))
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn another_account_signing_in_after_an_open_from_what_was_kept_replaces_the_old_lists()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let restarted = restart_apple(open_apple(dir.path()).await?).await?;
    let lists = lists_folder(Path::new(&*restarted.node.config_path), restarted.record.0);
    let (ui, binding) = restarted.open().await?;
    let tracks = |songs: Vec<(i32, String, AudioAvailability)>| -> Vec<String> {
        songs.into_iter().map(|(_, track, _)| track).collect()
    };
    assert_eq!(tracks(listed_songs(&ui).await?), ["i.s1", "i.s2", "i.s3"]);
    restarted.sign_in(USER_TOKEN).await?;

    // Other tokens may be another account's, which Apple does not say: the lists kept are read
    // again under them, and what the new account has replaces them, on disk too.
    restarted.fake.other_account.store(true, Ordering::SeqCst);
    restarted.sign_in(APPLE_OTHER_TOKEN).await?;
    wait_until("the pages to list what the new account has", || async {
        let listed = tracks(listed_songs(&ui).await?);
        Ok((listed == ["i.b-s1", "i.b-s2", "i.b-s3"]).then_some(()))
    })
    .await?;
    wait_until("the kept songs to be the new account's", || async {
        let kept = std::fs::read_to_string(lists.join("songs.json")).unwrap_or_default();
        Ok((kept.contains("i.b-s1") && !kept.contains("\"i.s1\"")).then_some(()))
    })
    .await?;
    assert!(Arc::ptr_eq(
        &restarted
            .node
            .services
            .apple_music_binding(restarted.record)
            .context("the binding")?,
        &binding
    ));
    restarted.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_apple_record_that_kept_nothing_opens_empty_and_its_reads_are_typed() -> Result<()> {
    use database::library_registry::{
        CatalogPlacement, LibraryRecord, LibraryRegistry, ProtocolAdapter,
    };

    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    // An Apple Music record this device never signed in to, so nothing of it is kept.
    let unsigned = LibraryRecord {
        record_id: LibraryRecordId(uuid::Uuid::new_v4()),
        alias: "Never signed in".into(),
        library_id: None,
        protocol: ProtocolAdapter::AppleMusic,
        fs: Vec::new(),
        catalog: CatalogPlacement::None,
        background_sync: false,
    };
    let unsigned_id = unsigned.record_id;
    LibraryRegistry::update(Path::new(&*apple.node.config_path), |registry| {
        registry.libraries.push(unsigned);
        Ok(())
    })
    .await?;
    let asked = apple.fake.total();
    let ui = open_registered_library_record(apple.node.clone(), unsigned_id.0.to_string()).await?;
    let binding = apple
        .node
        .services
        .apple_music_binding(unsigned_id)
        .context("the binding the open made")?;
    assert!(!binding.is_signed_in());
    assert_eq!(ui.library_id, Some(unsigned_id.0));

    let waits_for_sign_in = |error: anyhow::Error| {
        assert_eq!(
            request_failure_cause(&error),
            Some(RequestFailureCause::ServiceSignInRequired {
                record_id: unsigned_id.0.to_string(),
            }),
            "{error:#}"
        );
        assert_eq!(
            classify(&error),
            crate::messages::RequestErrorKind::PermissionDenied
        );
    };
    waits_for_sign_in(listed_songs(&ui).await.err().context("the songs")?);
    waits_for_sign_in(
        dispatch_gui_request(&ui, &GetMediaFilesCountRequest { call_id: None })
            .await
            .err()
            .context("the song count")?,
    );
    // The home has nothing to show, and says what it waits for.
    waits_for_sign_in(home_names(&ui).await.err().context("the home")?);

    // Offline, the same reads fail as every request does offline.
    apple.node.services.reach_changed(
        crate::apple_music_binding::APPLE_MUSIC_HOST,
        Reach::offline(HashSet::new()),
    );
    let error = listed_songs(&ui).await.err().context("the songs offline")?;
    assert_eq!(
        classify(&error),
        crate::messages::RequestErrorKind::Unavailable,
        "{error:#}"
    );
    assert_eq!(apple.fake.total(), asked);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unreachable_library_gives_no_color_and_an_unsigned_apple_item_waits_for_it()
-> Result<()> {
    use database::credentials::{Credential, Secret};
    use database::library_registry::{
        CatalogPlacement, LibraryRecord, LibraryRegistry, ProtocolAdapter, SubsonicOpenMode,
    };
    use std::sync::atomic::AtomicUsize;

    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    // A Subsonic server that would be probed if a binding were made for its library.
    let hits = Arc::new(AtomicUsize::new(0));
    let counted = hits.clone();
    let server = Router::new().fallback(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        async { StatusCode::INTERNAL_SERVER_ERROR }
    });
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    let subsonic_server = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
    let credential = apple
        .node
        .credentials
        .put(Credential {
            username: "account".into(),
            secret: Secret::new(b"unused".to_vec()),
        })
        .await?;
    let record = |protocol| LibraryRecord {
        record_id: LibraryRecordId(uuid::Uuid::new_v4()),
        alias: "Closed service".into(),
        library_id: None,
        protocol,
        fs: Vec::new(),
        catalog: CatalogPlacement::None,
        background_sync: false,
    };
    let subsonic = record(ProtocolAdapter::SubsonicTranslate {
        endpoint,
        credential,
        mode: SubsonicOpenMode::PlayOnThisDevice,
    });
    // An Apple library this device is not signed in to.
    let unsigned = record(ProtocolAdapter::AppleMusic);
    let (subsonic_id, unsigned_id) = (subsonic.record_id, unsigned.record_id);
    LibraryRegistry::update(Path::new(&*apple.node.config_path), |registry| {
        registry.libraries.push(subsonic);
        registry.libraries.push(unsigned);
        Ok(())
    })
    .await?;
    let ui = Arc::clone(&apple.ui);
    let item = |record: LibraryRecordId| PlayingItemRef::LibraryTrack {
        library_id: record.0.to_string(),
        track: "t".into(),
    };

    // The server of a closed Subsonic library answers with a failure, which gives no color.
    let color = dispatch_gui_request(
        &ui,
        &GetPrimaryColorByTrackIdRequest {
            call_id: None,
            item: Some(item(subsonic_id)),
        },
    )
    .await?
    .context("color")?;
    assert_eq!(color.primary_color, None);
    // Reaching the library for its cover is the only connection made.
    let connections = hits.load(Ordering::SeqCst);
    assert!(connections > 0);
    // The color of an Apple item whose library is not signed in here fails as a typed failure,
    // without a lookup elsewhere.
    let color = dispatch_gui_request(
        &ui,
        &GetPrimaryColorByTrackIdRequest {
            call_id: None,
            item: Some(item(unsigned_id)),
        },
    )
    .await
    .err()
    .context("the color of an unsigned Apple item")?;
    assert_eq!(
        request_failure_cause(&color),
        Some(RequestFailureCause::ServiceSignInRequired {
            record_id: unsigned_id.0.to_string(),
        }),
        "{color:#}"
    );
    assert_eq!(hits.load(Ordering::SeqCst), connections);
    subsonic_server.abort();
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_like_of_an_item_whose_library_cannot_answer_fails_as_a_typed_failure() -> Result<()> {
    use database::library_registry::{
        CatalogPlacement, LibraryRecord, LibraryRegistry, ProtocolAdapter,
    };

    use crate::messages::{GetLikedRequest, RequestErrorKind};

    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    // An Apple library this device is not signed in to yet, as at startup before the
    // platform's silent sign-in, and a Subsonic library whose server cannot be reached.
    let unsigned = LibraryRecord {
        record_id: LibraryRecordId(uuid::Uuid::new_v4()),
        alias: "Not signed in".into(),
        library_id: None,
        protocol: ProtocolAdapter::AppleMusic,
        fs: Vec::new(),
        catalog: CatalogPlacement::None,
        background_sync: false,
    };
    let unsigned_id = unsigned.record_id;
    LibraryRegistry::update(Path::new(&*apple.node.config_path), |registry| {
        registry.libraries.push(unsigned);
        Ok(())
    })
    .await?;
    let subsonic = write_subsonic_record(&apple.node, "http://127.0.0.1:9".into()).await?;
    let item = |record: LibraryRecordId| {
        Some(PlayingItemRef::LibraryTrack {
            library_id: record.0.to_string(),
            track: "t1".into(),
        })
    };
    let ui = Arc::clone(&apple.ui);

    for params in [&ui, &apple.fixture.global_params] {
        let likes = |record| {
            let item = item(record);
            async move {
                [
                    dispatch_gui_request(
                        params,
                        &GetLikedRequest {
                            item: item.clone(),
                            call_id: None,
                        },
                    )
                    .await
                    .err(),
                    dispatch_gui_request(
                        params,
                        &SetLikedRequest {
                            item,
                            liked: true,
                            call_id: None,
                        },
                    )
                    .await
                    .err(),
                ]
            }
        };
        // The Apple item's library has no binding to ask before its sign-in, which the UI takes
        // up as such.
        for failure in likes(unsigned_id).await {
            let failure = failure.context("a like before the sign-in")?;
            assert_eq!(
                classify(&failure),
                RequestErrorKind::PermissionDenied,
                "{failure:#}"
            );
            assert_eq!(
                request_failure_cause(&failure),
                Some(RequestFailureCause::ServiceSignInRequired {
                    record_id: unsigned_id.0.to_string(),
                })
            );
        }
        // The Subsonic item's library is out of reach for now.
        for failure in likes(subsonic).await {
            let failure = failure.context("a like with the server out of reach")?;
            assert_eq!(
                classify(&failure),
                RequestErrorKind::Unavailable,
                "{failure:#}"
            );
        }
    }
    apple.finish().await
}

/// Keeps what the player hands to the platform hosts.
#[derive(Default)]
struct RecordedHost(Mutex<Vec<playback::hosted::HostCommand>>);

impl playback::hosted::HostBridge for RecordedHost {
    fn send(&self, command: playback::hosted::HostCommand) {
        self.0.lock().unwrap().push(command);
    }
}

impl RecordedHost {
    fn sent(&self) -> Vec<playback::hosted::HostCommand> {
        self.0.lock().unwrap().clone()
    }
}

/// A track of the fixture's own library, scanned from the bundled startup sound.
async fn local_track(fixture: &Fixture, name: &str) -> Result<PlayingItemRef> {
    use database::actions::file::file_uuids_by_id;

    let library = fixture.global_params.test_library();
    let target = Path::new(&library.media_root).join(name);
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg"),
        &target,
    )?;
    scan_audio_file(
        &library.fsio,
        &library.main_db,
        &fixture.global_params.node_id,
        Path::new(&library.media_root),
        &target,
        &library.library_sync.record_store(),
    )
    .await?;
    let id = media_files::Entity::find()
        .filter(Column::FileName.eq(name))
        .one(&*library.main_db)
        .await?
        .context("the scanned track")?
        .id;
    let uuid = *file_uuids_by_id(&library.main_db, &[id])
        .await?
        .get(&id)
        .context("the scanned track's uuid")?;
    Ok(PlayingItemRef::LibraryTrack {
        library_id: library.library_id.to_string(),
        track: uuid.to_string(),
    })
}

/// A load of `items` in place of the queue, from the first at `position` seconds.
fn load_items(
    items: Vec<PlayingItemRef>,
    play: crate::messages::LoadPlay,
    position: f64,
) -> crate::messages::LoadResolvedItemsRequest {
    crate::messages::LoadResolvedItemsRequest {
        items,
        start_index: Some(0),
        mode: crate::messages::PlaylistOperateMode::Replace,
        expected_revision: None,
        playback_mode: Some(crate::messages::PlaybackMode::Sequential),
        position_seconds: Some(position),
        play,
        call_id: None,
    }
}

/// Waits for the player to show what `check` asks for, and returns its status then.
async fn player_reaches(
    player: &Arc<tokio::sync::Mutex<dyn playback::player::Playable>>,
    check: impl Fn(&playback::player::PlayerStatus) -> bool,
) -> Result<playback::player::PlayerStatus> {
    let check = &check;
    wait_until("the player to reach the state", || async move {
        let status = player.lock().await.get_status();
        Ok(check(&status).then_some(status))
    })
    .await
}

/// Opens the fixture's own library in the UI in place of the Apple Music one.
async fn open_own_library(apple: &OpenApple) -> Result<()> {
    let registry = LibraryRegistry::load(Path::new(&*apple.node.config_path))?;
    let record = registry
        .libraries
        .iter()
        .find(|record| matches!(record.protocol, ProtocolAdapter::Native))
        .context("the fixture's own library")?;
    super::open_registered_library_record(apple.node.clone(), record.record_id.0.to_string())
        .await?;
    Ok(())
}

/// The library item `item` names.
fn library_item(item: &PlayingItemRef) -> Result<playback::player::PlayingItem> {
    let PlayingItemRef::LibraryTrack { library_id, track } = item else {
        anyhow::bail!("not a library item: {item:?}");
    };
    Ok(playback::player::PlayingItem::LibraryTrack {
        library_id: library_id.parse()?,
        track: track.clone(),
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apple_tracks_play_on_the_host_beside_local_ones_and_stay_on_this_device() -> Result<()> {
    use ::visualizer::VisualizerInputs;
    use playback::hosted::{HostAction, HostEvent};
    use playback::player::PlaybackState;

    use crate::apple_music_binding::APPLE_MUSIC_HOST;
    use crate::messages::{
        CastLibraryState, CastResolvedItemsRequest, LoadPlay, SessionSnapshotRequest,
    };

    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let local = local_track(&apple.fixture, "beside-apple.ogg").await?;
    let song = song_ref(&apple, "i.s1");
    let host = Arc::new(RecordedHost::default());
    let player = apple.fixture.global_params.player.clone();
    player.lock().await.set_host_bridge(host.clone());
    let inputs = || apple.node.visualizer().status().inputs;
    assert_eq!(inputs(), VisualizerInputs::Full);

    apple
        .run(&load_items(
            vec![song.clone(), local.clone()],
            LoadPlay::Play,
            0.0,
        ))
        .await?;
    // The host gets the Apple track by its key, from the start; the local track after it is
    // not the host's to play.
    let sent = host.sent();
    let [hand_over] = sent.as_slice() else {
        anyhow::bail!("one hand-over, not {sent:?}");
    };
    assert_eq!(hand_over.host, APPLE_MUSIC_HOST);
    assert!(
        matches!(
            hand_over.actions.as_slice(),
            [
                HostAction::Set { ids },
                HostAction::Skip { index: 0, offset_ms: 0 },
                HostAction::SetGain { .. },
                HostAction::Start,
            ] if ids == &["i.s1".to_owned()]
        ),
        "{hand_over:?}"
    );
    let apple_item = library_item(&song)?;
    player_reaches(&player, |status| {
        status.item.as_ref() == Some(&apple_item)
            && status.hosted_by.as_deref() == Some(APPLE_MUSIC_HOST)
            && status.state == PlaybackState::Playing
    })
    .await?;
    wait_until("the visualizer to lose its inputs", || async {
        Ok((inputs() == VisualizerInputs::None).then_some(()))
    })
    .await?;

    // The queue holds an Apple track, so it cannot be handed off or cast elsewhere.
    let snapshot = apple
        .run(&SessionSnapshotRequest { call_id: None })
        .await
        .err()
        .context("a snapshot of a queue with an Apple track")?;
    assert!(
        matches!(
            snapshot.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::Unavailable { .. })
        ),
        "{snapshot:#}"
    );
    let cast = |items: Vec<PlayingItemRef>| CastResolvedItemsRequest {
        fingerprint: "not-a-paired-renderer".into(),
        load: load_items(items, LoadPlay::Play, 0.0),
        call_id: None,
    };
    let refused = apple
        .run(&cast(vec![local.clone(), song.clone()]))
        .await
        .err()
        .context("a cast of an Apple track")?;
    assert_eq!(
        request_failure_cause(&refused),
        Some(RequestFailureCause::CastNotPossible {
            library_id: apple.record.0.to_string(),
            state: CastLibraryState::NotHeld,
        })
    );
    // A local track and a Subsonic one, whose library other devices may reach, are not refused
    // for that: the cast only fails to find the renderer.
    let subsonic = write_subsonic_record(&apple.node, "http://127.0.0.1:9".into()).await?;
    let subsonic_track = PlayingItemRef::LibraryTrack {
        library_id: subsonic.0.to_string(),
        track: "s1".into(),
    };
    for items in [vec![local.clone()], vec![subsonic_track]] {
        let unknown = apple
            .run(&cast(items))
            .await
            .err()
            .context("a cast to a renderer that is not paired")?;
        assert!(
            matches!(
                unknown.downcast_ref::<database::error::ExpectedError>(),
                Some(database::error::ExpectedError::NotFound { what: "renderer" })
            ),
            "{unknown:#}"
        );
    }

    // The host plays the track to its end, and the local track decodes here.
    let revision = hand_over.revision;
    player
        .lock()
        .await
        .deliver_host_event(HostEvent::JukeboxStatus {
            host: APPLE_MUSIC_HOST.into(),
            revision,
            current_index: Some(0),
            current_id: Some("i.s1".into()),
            playing: true,
            position_ms: 1_000,
        });
    player
        .lock()
        .await
        .deliver_host_event(HostEvent::JukeboxEnded {
            host: APPLE_MUSIC_HOST.into(),
            revision,
            index: None,
        });
    let local_item = library_item(&local)?;
    let playing = player_reaches(&player, |status| {
        status.item.as_ref() == Some(&local_item)
            && status.hosted_by.is_none()
            && status.state == PlaybackState::Playing
    })
    .await?;
    assert!(
        playing
            .path
            .as_deref()
            .is_some_and(|path| path.ends_with("beside-apple.ogg")),
        "{:?}",
        playing.path
    );
    let sent = host.sent();
    assert!(
        matches!(
            sent.last().map(|command| command.actions.as_slice()),
            Some([HostAction::Stop, HostAction::Clear])
        ),
        "{sent:?}"
    );
    wait_until("the visualizer to get its inputs back", || async {
        Ok((inputs() == VisualizerInputs::Full).then_some(()))
    })
    .await?;
    player.lock().await.stop();
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_player_bar_names_the_track_the_host_moves_on_to_by_itself() -> Result<()> {
    use playback::hosted::{HostAction, HostEvent};
    use playback::player::PlaybackState;

    use crate::apple_music_binding::APPLE_MUSIC_HOST;
    use crate::messages::{LoadPlay, PlaybackStatus};

    let dir = tempfile::tempdir()?;
    let (apple, heard) = open_apple_heard(dir.path()).await?;
    let first = song_ref(&apple, "i.s1");
    let second = song_ref(&apple, "i.s2");
    let host = Arc::new(RecordedHost::default());
    let player = apple.fixture.global_params.player.clone();
    player.lock().await.set_host_bridge(host.clone());
    let titles = || {
        heard
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == "PlaybackStatus")
            .filter_map(|(_, payload)| rinf::deserialize::<PlaybackStatus>(payload).ok())
            .map(|status| (status.title, status.duration))
            .collect::<Vec<_>>()
    };

    apple
        .run(&load_items(
            vec![first.clone(), second.clone()],
            LoadPlay::Play,
            0.0,
        ))
        .await?;
    let first_item = library_item(&first)?;
    player_reaches(&player, |status| {
        status.item.as_ref() == Some(&first_item) && status.state == PlaybackState::Playing
    })
    .await?;
    wait_until("the player bar to name the first track", || async {
        Ok(titles()
            .last()
            .is_some_and(|(title, _)| title.as_deref() == Some("First Light"))
            .then_some(()))
    })
    .await?;

    // The host holds the second track behind the first, and moves on to it by itself.
    let revision = wait_until("the host to hold the second track", || async {
        Ok(host.sent().iter().rev().find_map(|command| {
            command
                .actions
                .iter()
                .any(|action| match action {
                    HostAction::Set { ids } | HostAction::Add { ids } => {
                        ids.iter().any(|id| id == "i.s2")
                    }
                    _ => false,
                })
                .then_some(command.revision)
        }))
    })
    .await?;
    let latest = host
        .sent()
        .last()
        .map_or(revision, |command| command.revision);
    player
        .lock()
        .await
        .deliver_host_event(HostEvent::JukeboxStatus {
            host: APPLE_MUSIC_HOST.into(),
            revision: latest,
            current_index: Some(1),
            current_id: Some("i.s2".into()),
            playing: true,
            position_ms: 1_000,
        });
    let second_item = library_item(&second)?;
    player_reaches(&player, |status| {
        status.item.as_ref() == Some(&second_item) && status.index == Some(1)
    })
    .await?;
    wait_until("the player bar to name the second track", || async {
        Ok(titles()
            .last()
            .is_some_and(|(title, _)| title.as_deref() == Some("Second Wind"))
            .then_some(()))
    })
    .await?;
    player.lock().await.stop();
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_hosted_track_gives_way_to_a_local_one_and_the_visualizer_inputs_follow()
-> Result<()> {
    use ::visualizer::VisualizerInputs;
    use playback::hosted::{HostEvent, HostFailure};
    use playback::player::{PlaybackState, UnavailableReason};

    use crate::apple_music_binding::APPLE_MUSIC_HOST;
    use crate::messages::LoadPlay;

    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let local = local_track(&apple.fixture, "after-failure.ogg").await?;
    let inputs = || apple.node.visualizer().status().inputs;

    // A report without a track, as between two hosted tracks, leaves the inputs as they are.
    let file =
        Path::new(&apple.fixture.global_params.test_library().media_root).join("after-failure.ogg");
    for (path, expected) in [
        (
            Some(Path::new("hosted://appleMusic/i.s1")),
            VisualizerInputs::None,
        ),
        (None, VisualizerInputs::None),
        (
            Some(Path::new("hosted://appleMusic/i.s2")),
            VisualizerInputs::None,
        ),
        (Some(file.as_path()), VisualizerInputs::Full),
        (None, VisualizerInputs::Full),
    ] {
        apple.node.note_local_track(path, true)?;
        assert_eq!(inputs(), expected, "after {path:?}");
    }

    let host = Arc::new(RecordedHost::default());
    let player = apple.fixture.global_params.player.clone();
    player.lock().await.set_host_bridge(host.clone());
    let song = song_ref(&apple, "i.s1");
    apple
        .run(&load_items(
            vec![song.clone(), local.clone()],
            LoadPlay::Play,
            0.0,
        ))
        .await?;
    let revision = host.sent().first().context("the hand-over")?.revision;
    wait_until("the visualizer to lose its inputs", || async {
        Ok((inputs() == VisualizerInputs::None).then_some(()))
    })
    .await?;

    player
        .lock()
        .await
        .deliver_host_event(HostEvent::JukeboxItemFailed {
            host: APPLE_MUSIC_HOST.into(),
            revision,
            index: 0,
            reason: HostFailure::Unplayable,
        });
    let local_item = library_item(&local)?;
    let playing = player_reaches(&player, |status| {
        status.item.as_ref() == Some(&local_item) && status.state == PlaybackState::Playing
    })
    .await?;
    assert_eq!(
        playing.unavailable_reasons.get(&library_item(&song)?),
        Some(&UnavailableReason::TrackGone)
    );
    wait_until("the visualizer to get its inputs back", || async {
        Ok((inputs() == VisualizerInputs::Full).then_some(()))
    })
    .await?;
    player.lock().await.stop();
    apple.finish().await
}

/// What the UI was told `item` supports, last before the first status that names it.
async fn told_before_its_status(
    heard: &Heard,
    item: &PlayingItemRef,
) -> Result<crate::messages::PlayingItemCapabilities> {
    use crate::messages::{PlaybackStatus, PlayingItemCapabilities};

    let status = wait_until("a status that names the item", || async {
        Ok(heard
            .placed::<PlaybackStatus>("PlaybackStatus")
            .into_iter()
            .find(|(_, status)| status.item_request.as_ref() == Some(item))
            .map(|(place, _)| place))
    })
    .await?;
    heard
        .placed::<PlayingItemCapabilities>("PlayingItemCapabilities")
        .into_iter()
        .rev()
        .find(|(place, told)| *place < status && told.item.as_ref() == Some(item))
        .map(|(_, told)| told)
        .context("what the item supports, before its status")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_ui_hears_what_the_playing_item_supports_before_the_status_that_names_it() -> Result<()>
{
    use playback::hosted::HostEvent;

    use crate::apple_music_binding::APPLE_MUSIC_HOST;
    use crate::messages::LoadPlay;

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
    let local = local_track(&apple.fixture, "after-apple.ogg").await?;
    let song = song_ref(&apple, "i.s1");
    let host = Arc::new(RecordedHost::default());
    let player = apple.fixture.global_params.player.clone();
    player.lock().await.set_host_bridge(host.clone());
    apple
        .run(&load_items(
            vec![song.clone(), local.clone()],
            LoadPlay::Play,
            0.0,
        ))
        .await?;

    // Apple Music answers the color of its track and whether it is liked in Rune, and keeps no
    // lyrics for it.
    let told = told_before_its_status(&heard, &song).await?;
    for capability in [
        told.get_primary_color_by_track_id_request,
        told.set_liked_request,
        told.get_liked_request,
    ] {
        assert_eq!(capability, RequestCapability::Available, "{told:?}");
    }
    assert_eq!(
        told.get_lyric_by_track_id_request,
        RequestCapability::Hidden,
        "{told:?}"
    );

    // The host plays its track to the end, and the local track after it has everything.
    let revision = host.sent().first().context("the hand-over")?.revision;
    player
        .lock()
        .await
        .deliver_host_event(HostEvent::JukeboxEnded {
            host: APPLE_MUSIC_HOST.into(),
            revision,
            index: None,
        });
    let told = told_before_its_status(&heard, &local).await?;
    for capability in [
        told.set_liked_request,
        told.get_liked_request,
        told.get_lyric_by_track_id_request,
        told.get_primary_color_by_track_id_request,
    ] {
        assert_eq!(capability, RequestCapability::Available, "{told:?}");
    }
    player.lock().await.stop();
    apple.finish().await
}

/// What the session declares for a file played by its path (`item_capabilities::file_slots`)
/// is what asking about it does: it has no liked state, and its lyrics and color are read
/// through the local library open in the UI, without which nothing about it is answered.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_played_by_its_path_is_answered_only_through_a_local_library() -> Result<()> {
    use crate::messages::{GetLikedRequest, RequestErrorKind};

    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let file = Some(PlayingItemRef::IndependentFile {
        path: Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/startup_0.ogg")
            .to_string_lossy()
            .into_owned(),
    });
    let asked = |params: Arc<crate::utils::GlobalParams>| {
        let file = file.clone();
        async move {
            [
                dispatch_gui_request(
                    &params,
                    &SetLikedRequest {
                        item: file.clone(),
                        liked: true,
                        call_id: None,
                    },
                )
                .await
                .map(|liked| liked.map(|liked| liked.liked)),
                dispatch_gui_request(
                    &params,
                    &GetLikedRequest {
                        item: file.clone(),
                        call_id: None,
                    },
                )
                .await
                .map(|liked| liked.map(|liked| liked.liked)),
                dispatch_gui_request(
                    &params,
                    &GetLyricByTrackIdRequest {
                        item: file.clone(),
                        call_id: None,
                    },
                )
                .await
                .map(|_| None),
                dispatch_gui_request(
                    &params,
                    &GetPrimaryColorByTrackIdRequest {
                        item: file,
                        call_id: None,
                    },
                )
                .await
                .map(|_| None),
            ]
        }
    };

    // Through the open Apple Music library, which has no local library, nothing is answered.
    let ui = Arc::clone(&apple.ui);
    for answer in asked(ui).await {
        let failure = answer.err().context("a request about the file")?;
        assert_ne!(
            classify(&failure),
            RequestErrorKind::Internal,
            "{failure:#}"
        );
    }
    // Through a local library, the file has no like to set or to show, and its lyrics and color
    // are read.
    let [set, get, lyrics, color] = asked(apple.fixture.global_params.clone()).await;
    let refused = set.err().context("a like of the file")?;
    assert_eq!(
        classify(&refused),
        RequestErrorKind::InvalidInput,
        "{refused:#}"
    );
    assert_eq!(get?, Some(false));
    lyrics?;
    color?;
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_paused_file_follows_the_library_the_ui_opens_and_closes() -> Result<()> {
    use crate::messages::PlayingItemCapabilities;

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
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/startup_0.ogg");
    let file = PlayingItemRef::IndependentFile {
        path: path.to_string_lossy().into_owned(),
    };
    // The file is queued by its path, as a file opened from outside a library is, and stays
    // paused: the player reports nothing more.
    apple
        .node
        .session()?
        .edit_queue(None, |player| {
            player.add_to_playlist_with_sources(
                vec![(
                    playback::player::PlayingItem::IndependentFile(path.to_string_lossy().into()),
                    playback::player::PlaybackSource::Local(path.clone()),
                )],
                playback::strategies::AddMode::AppendToEnd,
            );
            true
        })
        .await?;
    apple
        .run(&crate::messages::LoadRequest {
            index: 0,
            call_id: None,
        })
        .await?;
    let each = |told: &PlayingItemCapabilities| {
        [
            told.set_liked_request,
            told.get_liked_request,
            told.get_lyric_by_track_id_request,
            told.get_primary_color_by_track_id_request,
        ]
    };
    use RequestCapability::{Available, Hidden};
    // With a streaming library open there is no local library to read it through.
    let told = told_before_its_status(&heard, &file).await?;
    assert_eq!(each(&told), [Hidden; 4], "{told:?}");
    let last_told = || {
        heard
            .placed::<PlayingItemCapabilities>("PlayingItemCapabilities")
            .into_iter()
            .map(|(_, told)| told)
            .rfind(|told| told.item.as_ref() == Some(&file))
    };

    // The UI opens the local library: the file's lyrics and color can be read through it.
    let registry = LibraryRegistry::load(Path::new(&*apple.node.config_path))?;
    let record = registry
        .libraries
        .iter()
        .find(|record| matches!(record.protocol, ProtocolAdapter::Native))
        .context("the fixture's own library")?;
    let ui =
        super::open_registered_library_record(apple.node.clone(), record.record_id.0.to_string())
            .await?;
    wait_until("the UI to hear the file can be read", || async {
        Ok(last_told().filter(|told| each(told) == [Hidden, Hidden, Available, Available]))
    })
    .await?;

    // The UI closes it, and nothing about the file can be read again.
    apple.node.close_ui(&ui.main_token).await;
    wait_until("the UI to hear the file cannot be read", || async {
        Ok(last_told().filter(|told| each(told) == [Hidden; 4]))
    })
    .await?;
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_ui_hears_an_apple_item_supports_nothing_once_its_library_goes() -> Result<()> {
    use crate::messages::{LoadPlay, PlayingItemCapabilities, RemoveLibraryRecordRequest};

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
    let song = song_ref(&apple, "i.s1");
    apple
        .run(&load_items(vec![song.clone()], LoadPlay::Pause, 0.0))
        .await?;
    let told = told_before_its_status(&heard, &song).await?;
    assert_eq!(
        told.get_primary_color_by_track_id_request,
        RequestCapability::Available
    );
    let last_told = || {
        heard
            .placed::<PlayingItemCapabilities>("PlayingItemCapabilities")
            .into_iter()
            .map(|(_, told)| told)
            .rfind(|told| told.item.as_ref() == Some(&song))
    };

    // Another library is opened, and the Apple Music library is removed with its sign-in: no
    // sign-in brings it back, and nothing about the track can be answered.
    let subsonic = write_subsonic_record(&apple.node, "http://127.0.0.1:9".into()).await?;
    apple
        .node
        .open_service_catalog(
            Some(subsonic),
            Arc::new(LyricService::with_slots(subsonic.0, subsonic_slots())),
        )
        .await?;
    let removal = RemoveLibraryRecordRequest {
        call_id: None,
        record_id: apple.record.0.to_string(),
        delete_device_data: false,
    };
    <RemoveLibraryRecordRequest as crate::Signal>::handle(
        &removal,
        apple.node.clone(),
        None,
        &removal,
    )
    .await?;
    let told = wait_until("the UI to hear the track has nothing to answer", || async {
        Ok(last_told()
            .filter(|told| told.get_primary_color_by_track_id_request == RequestCapability::Hidden))
    })
    .await?;
    for capability in [
        told.set_liked_request,
        told.get_liked_request,
        told.get_lyric_by_track_id_request,
    ] {
        assert_eq!(capability, RequestCapability::Hidden, "{told:?}");
    }
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_ui_hears_each_change_of_the_visualizer_inputs_once() -> Result<()> {
    use crate::messages::{VisualizerInputs, VisualizerInputsChanged};

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
    let told = || {
        heard
            .placed::<VisualizerInputsChanged>("VisualizerInputsChanged")
            .into_iter()
            .map(|(_, changed)| changed.inputs)
            .collect::<Vec<_>>()
    };
    let before = told();
    // Whatever was sent while the library opened left this device's player feeding everything.
    assert_eq!(before.last(), Some(&VisualizerInputs::Full));

    let file = Path::new(&apple.fixture.global_params.test_library().media_root).join("a-file.ogg");
    for path in [
        Some(Path::new("hosted://appleMusic/i.s1")),
        None,
        Some(Path::new("hosted://appleMusic/i.s2")),
        Some(file.as_path()),
        None,
        Some(file.as_path()),
    ] {
        apple.node.note_local_track(path, path.is_some())?;
    }
    // The UI heard of the hosted track and of the file after it, and of nothing in between.
    assert_eq!(
        told()[before.len()..],
        [VisualizerInputs::None, VisualizerInputs::Full]
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restored_apple_tracks_keep_their_place_before_the_sign_in_and_are_named_after_it()
-> Result<()> {
    use crate::messages::{
        LoadPlay, PlaybackStatus, PlaylistUpdate, ServiceSignIn, SignInServiceLibraryRequest,
    };
    use crate::utils::sign_in_service_library_on_node;

    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let local = local_track(&apple.fixture, "after-restore.ogg").await?;
    let song = song_ref(&apple, "i.s1");
    let apple_item = library_item(&song)?;
    let at_its_place = |status: &playback::player::PlayerStatus| {
        status.item.as_ref() == Some(&apple_item)
            && status.index == Some(0)
            && status.position == Duration::from_secs(30)
            && status.ready
    };
    apple
        .run(&load_items(
            vec![song.clone(), song_ref(&apple, "i.s2"), local],
            LoadPlay::Pause,
            30.0,
        ))
        .await?;
    player_reaches(&apple.fixture.global_params.player, at_its_place).await?;

    // The app quits, and starts again with the hub not signed in to Apple Music.
    apple.node.shutdown().await?;
    let heard = Arc::new(Heard::default());
    let config = apple.fixture.global_params.config_path.clone();
    let node = node_on(&config, heard.clone(), &apple.fake.base).await?;
    assert!(node.services.apple_music_binding(apple.record).is_none());
    let player = node.session()?.player.clone();
    player_reaches(&player, at_its_place).await?;
    let settled = player.lock().await.status_after_pending().await;
    assert!(at_its_place(&settled), "{settled:?}");
    assert!(settled.unavailable_reasons.is_empty(), "{settled:?}");

    let decoded = |name: &str| -> Vec<Vec<u8>> {
        heard
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(heard, _)| heard == name)
            .map(|(_, payload)| payload.clone())
            .collect()
    };
    let named = || {
        decoded("PlaybackStatus")
            .iter()
            .filter_map(|payload| rinf::deserialize::<PlaybackStatus>(payload).ok())
            .any(|status| status.title.as_deref() == Some("First Light"))
    };
    let sign_in = || {
        let request = SignInServiceLibraryRequest {
            record_id: apple.record.0.to_string(),
            sign_in: ServiceSignIn::Platform {
                user_token: USER_TOKEN.into(),
                developer_token: Some(DEVELOPER_TOKEN.into()),
            },
            call_id: None,
        };
        let node = node.clone();
        async move { sign_in_service_library_on_node(&request, &node).await }
    };

    let lookups = || {
        apple
            .fake
            .seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(target, _)| target.starts_with("/v1/me/library/songs?ids="))
            .map(|(target, _)| target.clone())
            .collect::<Vec<_>>()
    };

    // A first sign-in finds Apple unable to name the songs: the binding is there, but the
    // tracks stay unnamed.
    *apple.fake.respond_path.lock().unwrap() =
        Some(("/v1/me/library/songs".into(), 404, "{}".into()));
    let updates = decoded("PlaylistUpdate").len();
    sign_in().await?;
    // This sign-in asks Apple for the queue's songs together and for the playing song on its
    // own, for the player bar. Both are waited for, so neither counts as a read of the next
    // sign-in.
    wait_until("the first sign-in's reads", || async {
        let lookups = lookups();
        let asked = |ids: &str| {
            lookups
                .iter()
                .any(|target| target.starts_with(&format!("/v1/me/library/songs?ids={ids}&")))
        };
        Ok((asked("i.s1,i.s2") && asked("i.s1")).then_some(()))
    })
    .await?;
    wait_until("the queue to be sent after the first sign-in", || async {
        Ok((decoded("PlaylistUpdate").len() > updates).then_some(()))
    })
    .await?;
    assert!(!named());
    *apple.fake.respond_path.lock().unwrap() = None;

    // Once the app signs the hub in again, the player bar and the queue name the tracks,
    // which are read together.
    let before = lookups().len();
    sign_in().await?;
    wait_until("the player bar to name the track", || async {
        Ok(named().then_some(()))
    })
    .await?;
    wait_until("the queue to name the tracks", || async {
        Ok(decoded("PlaylistUpdate")
            .iter()
            .filter_map(|payload| rinf::deserialize::<PlaylistUpdate>(payload).ok())
            .any(|update| {
                let titles: Vec<&str> = update.items.iter().map(|row| row.title.as_str()).collect();
                titles.starts_with(&["First Light", "Second Wind"])
                    && update.items.iter().all(|row| row.unavailable.is_none())
            })
            .then_some(()))
    })
    .await?;
    let lookups = lookups();
    assert_eq!(lookups.len(), before + 1, "{lookups:?}");
    assert!(
        lookups[before].starts_with("/v1/me/library/songs?ids=i.s1,i.s2"),
        "{lookups:?}"
    );
    assert!(at_its_place(&player.lock().await.get_status()));
    apple.server.abort();
    node.shutdown().await
}

/// How many bytes this device sends the renderer behind `proxy` for what `sends` sends, measured
/// up to the answer of `barrier`, a request known to reach the renderer on the same link: once it
/// is answered, every byte sent before it has passed the proxy. The bytes of `barrier` itself are
/// not counted.
async fn bytes_sent<S, B>(
    proxy: &super::player_session_tests::TestTcpProxy,
    sends: S,
    barrier: impl AsyncFn() -> Result<B>,
) -> Result<usize>
where
    S: Future,
{
    let before = proxy.request_bytes();
    barrier().await?;
    let barrier_bytes = proxy.request_bytes() - before;
    let before = proxy.request_bytes();
    sends.await;
    barrier().await?;
    Ok(proxy.request_bytes() - before - barrier_bytes)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apple_items_stay_on_this_device_while_a_renderer_is_attached() -> Result<()> {
    use super::player_session_tests::TestTcpProxy;
    use crate::messages::{
        CastLibraryState, ListRenderersRequest, LoadPlay, RendererAvailability, RequestErrorKind,
        SessionSnapshotRequest,
    };
    use crate::request_owner::CurrentPlayer;
    use crate::server::api::fetch_device_info_at_port;

    let controller_dir = tempfile::tempdir()?;
    let renderer_dir = tempfile::tempdir()?;
    let apple = open_apple(controller_dir.path()).await?;
    let renderer = super::fixture(renderer_dir.path()).await?;
    let renderer_address = super::paired_full(&renderer, &apple.fixture).await?;
    // Only the renderer's link passes the proxy, through which the test sees what this device
    // sends the renderer.
    apple.fixture.stop_worker().await?;
    renderer.stop_worker().await?;
    let proxy = TestTcpProxy::new(renderer_address).await?;
    let renderer_info = super::info(&renderer).await?;
    let validator = &apple.fixture.global_params.cert_validator;
    validator
        .read()
        .await
        .replace_hosts_for_fingerprint(
            &renderer_info.fingerprint,
            vec![format!("127.0.0.1:{}", proxy.listen_addr.port())],
        )
        .await?;
    let client_validator = validator.read().await.clone();
    wait_until("the renderer to answer", || async {
        let ready = fetch_device_info_at_port(
            "127.0.0.1",
            renderer_address.port(),
            Arc::new(Arc::new(client_validator.clone()).into_client_config()),
        )
        .await
        .is_ok();
        Ok(ready.then_some(()))
    })
    .await?;
    let apple_library = apple.record.0.to_string();

    // The renderer is not asked about the Apple library: it is answered here, ahead of a
    // library the renderer would be asked about.
    let local_library = apple
        .fixture
        .global_params
        .library_id
        .context("the local library")?
        .to_string();
    let renderers = apple
        .run(&ListRenderersRequest {
            call_id: None,
            library_ids: vec![local_library, apple_library.clone()],
        })
        .await?
        .renderers;
    let candidate = renderers
        .iter()
        .find(|candidate| candidate.fingerprint == renderer_info.fingerprint)
        .context("the renderer among the candidates")?;
    assert_eq!(candidate.availability, RendererAvailability::CantPlay);
    assert_eq!(
        candidate.blocked_library_id.as_deref(),
        Some(apple_library.as_str())
    );
    assert_eq!(candidate.library_state, Some(CastLibraryState::NotHeld));

    // An open service library keeps the player on this device, so another library is opened
    // first. With the renderer as the player, a load of an Apple track, as from the queue,
    // never leaves this device.
    open_own_library(&apple).await?;
    apple
        .node
        .attach_renderer(&renderer_info.fingerprint)
        .await?;
    assert!(matches!(
        apple.node.current_player()?,
        CurrentPlayer::RunePeer { link: Some(_), .. }
    ));
    let refused = apple
        .run(&load_items(
            vec![song_ref(&apple, "i.s1")],
            LoadPlay::Play,
            0.0,
        ))
        .await
        .err()
        .context("a load of an Apple track on the renderer")?;
    assert_eq!(
        request_failure_cause(&refused),
        Some(RequestFailureCause::CastNotPossible {
            library_id: apple_library,
            state: CastLibraryState::NotHeld,
        })
    );
    // Any other request that would name the library on the renderer, as a mix whose fallback
    // items are Apple tracks, is refused as unavailable, not as a cast.
    let remote = crate::request_owner::Implementation::RunePeer {
        link: apple.node.renderer_link()?,
        reconnecting: None,
    };
    let mix = ResolveMixQueryRequest {
        queries: Vec::new(),
        hint_position: -1,
        initial_playback_item: None,
        instantly_play: false,
        fallback_playing_items: vec![song_ref(&apple, "i.s1")],
        excluded: ExcludedItems::default(),
        call_id: None,
    };
    let unavailable = remote
        .run(&apple.fixture.global_params, &mix)
        .await
        .err()
        .context("a mix naming an Apple track asked of the renderer")?;
    assert!(
        matches!(
            unavailable.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::Unavailable {
                what: "library kept on this device"
            })
        ),
        "{unavailable:#}"
    );
    // The color of an Apple track names the library too, but it is answered here, from the
    // covers the library keeps, which hold none for it yet.
    let color = GetPrimaryColorByTrackIdRequest {
        item: Some(song_ref(&apple, "i.s1")),
        call_id: None,
    };
    let to_renderer = async || {
        remote
            .run(
                &apple.fixture.global_params,
                &SessionSnapshotRequest { call_id: None },
            )
            .await
    };
    let mut answered = None;
    let sent = bytes_sent(
        &proxy,
        async {
            answered = Some(remote.run(&apple.fixture.global_params, &color).await);
        },
        to_renderer,
    )
    .await?;
    let answered = answered
        .context("the color asked")??
        .context("the color of an Apple track asked of the renderer")?;
    assert_eq!(answered.primary_color, None);
    assert_eq!(
        sent, 0,
        "the color of an Apple track was asked of the renderer"
    );
    // While the remote library reconnects, or has no link, an Apple track is still answered
    // here, and a request that is not routed by item still waits for the remote library.
    for (link, reconnecting, expected) in [
        (None, Some("Renderer".to_owned()), "reconnecting"),
        (None, None, "unavailable"),
    ] {
        let away = crate::request_owner::Implementation::RunePeer { link, reconnecting };
        let answered = away
            .run(&apple.fixture.global_params, &color)
            .await?
            .context("the color of an Apple track while the remote library is away")?;
        assert_eq!(answered.primary_color, None);
        let waiting = away
            .run(&apple.fixture.global_params, &mix)
            .await
            .err()
            .context("a mix while the remote library is away")?;
        let kind = match waiting.downcast_ref::<ExpectedRequestError>() {
            Some(ExpectedRequestError::Reconnecting { .. }) => "reconnecting",
            Some(ExpectedRequestError::Unavailable { .. }) => "unavailable",
            _ => "other",
        };
        assert_eq!(kind, expected, "{waiting:#}");
    }
    // A Subsonic track goes to the renderer, which refuses it for its own reasons.
    let subsonic = write_subsonic_record(&apple.node, "http://127.0.0.1:9".into()).await?;
    let subsonic_track = PlayingItemRef::LibraryTrack {
        library_id: subsonic.0.to_string(),
        track: "s1".into(),
    };
    let forwarded = apple
        .run(&load_items(
            vec![subsonic_track.clone()],
            LoadPlay::Play,
            0.0,
        ))
        .await
        .err()
        .context("a load of a Subsonic track on the renderer")?;
    assert_eq!(
        classify(&forwarded),
        RequestErrorKind::InvalidInput,
        "{forwarded:#}"
    );
    assert!(
        matches!(
            forwarded.downcast_ref::<ExpectedRequestError>(),
            Some(ExpectedRequestError::Remote { .. })
        ),
        "{forwarded:#}"
    );

    apple.node.detach_renderer().await?;

    // With the renderer's library open here, browsing it lists the caller's queue, from which
    // the Apple track is left out before it goes; the rest goes, and the page still answers.
    let renderer_library = renderer
        .global_params
        .library_id
        .context("the renderer's library")?;
    renderer
        .global_params
        .node
        .set_open_local_catalog(None, renderer_library)
        .await?;
    let remote =
        super::open_remote_control_record(apple.node.clone(), &renderer_info.fingerprint).await?;
    let queue_query = vec![MixQuery {
        operator: "lib::queue".into(),
        parameter: "true".into(),
    }];
    let page = |items: Vec<PlayingItemRef>| crate::messages::MixQueryRequest {
        queries: queue_query.clone(),
        cursor: 0,
        page_size: 30,
        bake_cover_arts: false,
        queue: CallerQueue { items },
        random_seed: None,
        call_id: None,
    };
    let mix = |items: Vec<PlayingItemRef>| ResolveMixQueryRequest {
        queries: queue_query.clone(),
        hint_position: -1,
        initial_playback_item: None,
        instantly_play: false,
        fallback_playing_items: Vec::new(),
        excluded: ExcludedItems { items },
        call_id: None,
    };
    let to_renderer =
        async || dispatch_gui_request(&remote, &SessionSnapshotRequest { call_id: None }).await;
    let with_apple = vec![song_ref(&apple, "i.s1"), subsonic_track.clone()];
    let without_apple = vec![subsonic_track.clone()];
    // What goes for a request that lists the Apple track is what goes for one that lists only the
    // rest, and both answer.
    let mut sizes = Vec::new();
    for items in [with_apple.clone(), without_apple.clone()] {
        let mut answered = None;
        let sent = bytes_sent(
            &proxy,
            async {
                answered = Some(dispatch_gui_request(&remote, &page(items)).await);
            },
            to_renderer,
        )
        .await?;
        answered
            .context("the page asked")??
            .context("a page of the renderer's library")?;
        sizes.push(sent);
    }
    assert!(sizes[1] > 0, "the page did not reach the renderer");
    assert_eq!(sizes[0], sizes[1], "the Apple track went with the page");
    let mut sizes = Vec::new();
    for items in [with_apple.clone(), without_apple] {
        let mut answered = None;
        let sent = bytes_sent(
            &proxy,
            async {
                answered = Some(dispatch_gui_request(&remote, &mix(items)).await);
            },
            to_renderer,
        )
        .await?;
        answered
            .context("the mix asked")??
            .context("a mix of the renderer's library")?;
        sizes.push(sent);
    }
    assert!(sizes[1] > 0, "the mix did not reach the renderer");
    assert_eq!(sizes[0], sizes[1], "the Apple track went with the mix");

    // With the renderer's library open here, what an Apple track's library declares for its
    // items is decided here: a like is kept in Rune on this device, it keeps no lyrics, which
    // are refused, and there is no cover file to take a color from.
    let mut answers = Vec::new();
    let sent = bytes_sent(
        &proxy,
        async {
            answers.push(
                dispatch_gui_request(
                    &remote,
                    &SetLikedRequest {
                        item: Some(song_ref(&apple, "i.s1")),
                        liked: true,
                        call_id: None,
                    },
                )
                .await
                .map(drop),
            );
            answers.push(
                dispatch_gui_request(
                    &remote,
                    &GetLyricByTrackIdRequest {
                        item: Some(song_ref(&apple, "i.s1")),
                        call_id: None,
                    },
                )
                .await
                .map(drop),
            );
        },
        to_renderer,
    )
    .await?;
    let mut answers = answers.into_iter();
    answers
        .next()
        .context("the like asked")?
        .context("a like of an Apple track with the renderer's library open")?;
    let kept = std::fs::read_to_string(records_of(&apple, apple.record).join("likes.json"))?;
    assert!(kept.contains("\"i.s1\""), "{kept}");
    let refused = answers
        .next()
        .context("the lyrics asked")?
        .err()
        .context("the lyrics of an Apple track with the renderer's library open")?;
    assert!(unsupported(&refused), "{refused:#}");
    let mut color = None;
    let sent = sent
        + bytes_sent(
            &proxy,
            async {
                color = Some(
                    dispatch_gui_request(
                        &remote,
                        &GetPrimaryColorByTrackIdRequest {
                            item: Some(song_ref(&apple, "i.s1")),
                            call_id: None,
                        },
                    )
                    .await,
                );
            },
            to_renderer,
        )
        .await?;
    let color = color
        .context("the color asked")??
        .context("the color of an Apple track with the renderer's library open")?;
    assert_eq!(color.primary_color, None);
    assert_eq!(
        sent, 0,
        "a request about an Apple track reached the renderer"
    );
    // A like of a track of a library the renderer may hold goes to the renderer, which answers
    // for itself.
    let mut peer_answer = None;
    let sent = bytes_sent(
        &proxy,
        async {
            for liked in [true, false] {
                let _ = dispatch_gui_request(
                    &remote,
                    &SetLikedRequest {
                        item: Some(subsonic_track.clone()),
                        liked,
                        call_id: None,
                    },
                )
                .await;
            }
            peer_answer = Some(
                dispatch_gui_request(
                    &remote,
                    &crate::messages::GetLikedRequest {
                        item: Some(subsonic_track.clone()),
                        call_id: None,
                    },
                )
                .await,
            );
        },
        to_renderer,
    )
    .await?;
    assert!(
        sent > 0,
        "the likes of a Subsonic track did not reach the renderer"
    );
    if let Some(Err(error)) = peer_answer {
        assert!(
            matches!(
                error.downcast_ref::<ExpectedRequestError>(),
                Some(ExpectedRequestError::Remote { .. })
            ),
            "{error:#}"
        );
    }

    renderer.global_params.node.shutdown().await?;
    apple.finish().await
}

/// An Apple library whose node broadcaster is heard.
async fn open_apple_heard(dir: &Path) -> Result<(OpenApple, Arc<Heard>)> {
    let heard = Arc::new(Heard::default());
    let apple = open_apple_with(
        dir,
        AppleOptions {
            heard: Some(heard.clone()),
            ..AppleOptions::default()
        },
    )
    .await?;
    Ok((apple, heard))
}

const CONTENT_CHANGED: &str = "LibraryContentChanged";

async fn open_big_apple(dir: &Path, extra_songs: usize) -> Result<OpenApple> {
    open_apple_with(
        dir,
        AppleOptions {
            extra_songs,
            ..AppleOptions::default()
        },
    )
    .await
}

/// Waits until the fake has been asked for something whose path and query start with `prefix`.
async fn fake_asked(apple: &OpenApple, prefix: &'static str, times: usize) -> Result<()> {
    wait_until("the fake Apple API to be asked", || async {
        Ok((apple.fake.count(prefix) >= times).then_some(()))
    })
    .await
}

async fn collection_names(
    apple: &OpenApple,
    collection_type: CollectionType,
    ids: &[i32],
) -> Result<Vec<String>> {
    Ok(apple
        .run(&FetchCollectionByIdsRequest {
            call_id: None,
            collection_type,
            bake_cover_arts: false,
            ids: ids.to_vec(),
        })
        .await?
        .result
        .into_iter()
        .map(|collection| collection.name)
        .collect())
}

/// The names and ids of the artists listed in the groups `titles`.
async fn listed_artists(apple: &OpenApple, titles: &[&str]) -> Result<Vec<(String, i32)>> {
    Ok(apple
        .run(&FetchCollectionGroupsRequest {
            call_id: None,
            collection_type: CollectionType::Artist,
            bake_cover_arts: false,
            group_titles: titles.iter().map(|title| (*title).to_owned()).collect(),
        })
        .await?
        .groups
        .into_iter()
        .flat_map(|group| group.collections)
        .map(|collection| (collection.name, collection.id))
        .collect())
}

/// The titles of the songs of an artist's page.
async fn artist_songs(apple: &OpenApple, artist: i32) -> Result<Vec<String>> {
    Ok(apple
        .run(&mix("lib::artist", artist))
        .await?
        .files
        .into_iter()
        .map(|file| file.title)
        .collect())
}

async fn library_album_songs(apple: &OpenApple, remote: &str) -> Result<Vec<String>> {
    let album = apple.binding.intern(AppleIdKind::Album, remote)?;
    Ok(apple
        .run(&mix("lib::album", album))
        .await?
        .files
        .into_iter()
        .map(|file| file.title)
        .collect())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_library_album_page_is_answered_from_the_song_list() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    assert_eq!(
        library_album_songs(&apple, "l.alb1").await?,
        ["First Light", "Second Wind"]
    );
    assert_eq!(apple.fake.count("/v1/me/library/albums/l.alb1"), 0);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_album_page_is_read_from_apple_when_the_song_list_lacks_its_songs() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            unlisted_songs: true,
            ..AppleOptions::default()
        },
    )
    .await?;
    assert_eq!(
        library_album_songs(&apple, "l.alb1").await?,
        ["First Light", "Second Wind"]
    );
    assert!(apple.fake.count("/v1/me/library/albums/l.alb1") > 0);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn artists_credited_together_are_listed_one_by_one_with_pictures_of_their_own() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            credited: true,
            ..AppleOptions::default()
        },
    )
    .await?;

    let summary = apple
        .run(&FetchCollectionGroupSummaryRequest {
            call_id: None,
            collection_type: CollectionType::Artist,
        })
        .await?;
    let groups: Vec<_> = summary
        .groups
        .iter()
        .map(|group| (group.group_title.as_str(), group.count))
        .collect();
    // "Artist Two" is no library artist here, only credited by a song.
    assert_eq!(groups, [("A", 2), ("G", 1)]);
    let listed = listed_artists(&apple, &["A", "G"]).await?;
    let names: Vec<&str> = listed.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["Artist One", "Artist Two", "Guest"]);
    let (one, guest) = (listed[0].1, listed[2].1);
    // Each shows the catalog artist of its name, though Apple names "Artist One" alone for
    // one of the credits of both.
    let cover = |id| {
        apple
            .binding
            .collection_info(AppleIdKind::Artist, id)
            .and_then(|info| info.cover)
    };
    let photo = |name: &str| {
        Some(format!(
            "{}/art/{name}_Photo/600x600bb.jpg",
            apple.fake.base
        ))
    };
    assert_eq!(cover(one), photo("Artist_One"));
    assert_eq!(cover(guest), photo("Guest"));
    assert_eq!(
        collection_names(&apple, CollectionType::Artist, &[one, guest]).await?,
        ["Artist One", "Guest"]
    );

    // An artist's page has the songs Apple's artists name them for, each once, by album name
    // as no album has a year, and it is answered from the song list. Apple names only
    // "Artist One" for the credit "Artist One & Guest", so that song is not Guest's.
    let reads = apple.fake.total();
    assert_eq!(
        artist_songs(&apple, one).await?,
        ["First Light", "Second Wind", "Duet", "Guest Spot"]
    );
    assert_eq!(artist_songs(&apple, guest).await?, ["Guest Spot"]);
    assert_eq!(apple.fake.total(), reads);

    // A library artist the UI was given before still opens, as that credit alone.
    let duo = apple.binding.intern(AppleIdKind::Artist, "r.duo")?;
    assert_eq!(
        collection_names(&apple, CollectionType::Artist, &[duo]).await?,
        ["Artist One & Guest"]
    );
    assert_eq!(artist_songs(&apple, duo).await?, ["Duet"]);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_artist_only_a_song_credits_is_listed_linked_and_opens_from_the_song_list() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            solo: true,
            ..AppleOptions::default()
        },
    )
    .await?;

    let listed = listed_artists(&apple, &["A", "S"]).await?;
    let names: Vec<&str> = listed.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["Artist One", "Artist Two", "Solo Voice"]);
    let solo = listed[2].1;

    // The song's details link to that entry, never to id 0.
    let song = apple.binding.intern(AppleIdKind::LibrarySong, "i.solo")?;
    let parsed = apple
        .run(&FetchParsedMediaFileRequest {
            call_id: None,
            id: song,
        })
        .await?;
    assert_eq!(parsed.artists[0].name, "Solo Voice");
    assert_eq!(parsed.artists[0].id, solo);
    assert_ne!(solo, 0);

    // The entry's page lists the song, from the song list.
    let reads = apple.fake.total();
    assert_eq!(artist_songs(&apple, solo).await?, ["Alone"]);
    assert_eq!(apple.fake.total(), reads);
    apple.finish().await
}

fn catalog_credited() -> AppleOptions {
    AppleOptions {
        catalog_credits: true,
        ..AppleOptions::default()
    }
}

/// Every artist of the Artists page, by name, with its id.
async fn all_artists(apple: &OpenApple) -> Result<Vec<(String, i32)>> {
    let titles: Vec<String> = apple
        .run(&FetchCollectionGroupSummaryRequest {
            call_id: None,
            collection_type: CollectionType::Artist,
        })
        .await?
        .groups
        .into_iter()
        .map(|group| group.group_title)
        .collect();
    let titles: Vec<&str> = titles.iter().map(String::as_str).collect();
    let mut listed = listed_artists(apple, &titles).await?;
    listed.sort();
    Ok(listed)
}

fn names(listed: &[(String, i32)]) -> Vec<&str> {
    listed.iter().map(|(name, _)| name.as_str()).collect()
}

fn artist_named(listed: &[(String, i32)], name: &str) -> Result<i32> {
    listed
        .iter()
        .find(|(listed, _)| listed == name)
        .map(|(_, id)| *id)
        .with_context(|| format!("the artist {name}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn artists_are_told_apart_by_their_catalog_artists_whatever_their_credits_spell() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), catalog_credited()).await?;
    let listed = all_artists(&apple).await?;
    assert_eq!(
        names(&listed),
        [
            "Ann",
            "Artist One",
            "Artist Two",
            "Bob",
            "Hiroyuki Sawano",
            "サワノヒロユキ & Cara"
        ]
    );
    let remote = |name: &str| -> Result<String> {
        apple
            .binding
            .resolve(AppleIdKind::Artist, artist_named(&listed, name)?)
    };
    assert_eq!(remote("Ann")?, "artist:400");
    assert_eq!(remote("Hiroyuki Sawano")?, "artist:156");
    assert_eq!(remote("Artist One")?, "artist:300");
    // Apple names no catalog artist for these, so their credits tell them, whole.
    assert_eq!(
        remote("サワノヒロユキ & Cara")?,
        "name:サワノヒロユキ & cara"
    );
    assert_eq!(remote("Artist Two")?, "name:artist two");

    // "Ann & Bob" is listed under both its catalog artists.
    let ann = artist_named(&listed, "Ann")?;
    assert_eq!(artist_songs(&apple, ann).await?, ["Together", "Reversed"]);
    let bob = artist_named(&listed, "Bob")?;
    assert_eq!(artist_songs(&apple, bob).await?, ["Together"]);
    // "サワノヒロユキ" is the catalog's "Hiroyuki Sawano", named so. The song without a catalog id
    // stays under its whole credit: its text is not split to find "Hiroyuki Sawano" in it.
    let sawano = artist_named(&listed, "Hiroyuki Sawano")?;
    assert_eq!(
        artist_songs(&apple, sawano).await?,
        ["Kana Song", "English Song", "Reversed"]
    );
    let whole = artist_named(&listed, "サワノヒロユキ & Cara")?;
    assert_eq!(artist_songs(&apple, whole).await?, ["No Catalog"]);
    // A song's details link to its catalog artists, by the ids the list gives them.
    let parsed = apple
        .run(&FetchParsedMediaFileRequest {
            call_id: None,
            id: apple.binding.intern(AppleIdKind::LibrarySong, "i.rev")?,
        })
        .await?;
    assert_eq!(
        parsed
            .artists
            .iter()
            .map(|artist| (artist.id, artist.name.as_str()))
            .collect::<Vec<_>>(),
        [(sawano, "Hiroyuki Sawano"), (ann, "Ann")]
    );
    // The catalog was asked once for every song with a catalog id, and not again.
    let asked = apple.fake.count("/v1/catalog/it/songs?");
    all_artists(&apple).await?;
    artist_songs(&apple, sawano).await?;
    assert_eq!(apple.fake.count("/v1/catalog/it/songs?"), asked);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_catalog_artists_of_a_large_library_are_read_several_batches_at_once() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_big_apple(dir.path(), 1000).await?;
    assert_eq!(apple.fake.count("/v1/catalog/it/songs?"), 0);
    *apple.fake.delay_path.lock().unwrap() = Some((
        "/v1/catalog/it/songs".into(),
        std::time::Duration::from_millis(300),
    ));
    let listed = all_artists(&apple).await?;
    assert_eq!(names(&listed), ["Artist One", "Artist Two"]);
    // The songs were asked about in several batches, more than one at a time, and every answer
    // was kept.
    let batches = apple.fake.count("/v1/catalog/it/songs?");
    assert!(batches >= 4, "{batches} batches");
    assert!(
        apple
            .fake
            .most_catalog_songs_in_flight
            .load(Ordering::SeqCst)
            > 1
    );
    let kept = apple
        .binding
        .kept_song_artists()
        .await
        .context("the kept catalog artists")?;
    let songs = apple.binding.known_songs().context("the songs")?;
    assert!(kept.missing(songs.songs()).is_empty());
    // So a listing again asks about none.
    all_artists(&apple).await?;
    assert_eq!(apple.fake.count("/v1/catalog/it/songs?"), batches);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_first_artist_list_waits_a_while_for_the_catalog_then_follows_it() -> Result<()> {
    use crate::apple_music_binding::FIRST_SONG_ARTISTS_WAIT;

    let heard = Arc::new(Heard::default());
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            heard: Some(heard.clone()),
            ..catalog_credited()
        },
    )
    .await?;
    assert_eq!(apple.fake.count("/v1/catalog/it/songs?"), 0);
    let release = Arc::new(tokio::sync::Notify::new());
    *apple.fake.hold_once.lock().unwrap() = Some(("/v1/catalog/it/songs".into(), release.clone()));
    let by_credits = [
        "Ann & Bob",
        "Artist One",
        "Artist Two",
        "Hiroyuki Sawano",
        "Sawano Hiroyuki & Ann",
        "サワノヒロユキ",
        "サワノヒロユキ & Cara",
    ];
    // Apple does not answer in time, so the first list tells its artists apart by their
    // credits once it waited its while.
    let started = std::time::Instant::now();
    assert_eq!(names(&all_artists(&apple).await?), by_credits);
    let waited = started.elapsed();
    assert!(
        waited >= FIRST_SONG_ARTISTS_WAIT && waited < FIRST_SONG_ARTISTS_WAIT * 3,
        "waited {waited:?}"
    );
    // Later lists do not wait again.
    let started = std::time::Instant::now();
    assert_eq!(names(&all_artists(&apple).await?), by_credits);
    assert!(started.elapsed() < FIRST_SONG_ARTISTS_WAIT);
    assert_eq!(apple.fake.count("/v1/catalog/it/songs?"), 1);

    // Once Apple answers, the read is kept, the UI is told, and the list follows.
    let told = heard.count(CONTENT_CHANGED);
    release.notify_one();
    wait_until("the artists to be told apart by the catalog", || async {
        let listed = all_artists(&apple).await?;
        Ok((names(&listed)
            == [
                "Ann",
                "Artist One",
                "Artist Two",
                "Bob",
                "Hiroyuki Sawano",
                "サワノヒロユキ & Cara",
            ])
        .then_some(()))
    })
    .await?;
    told_after(&heard, told).await?;
    assert_eq!(apple.fake.count("/v1/catalog/it/songs?"), 1);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_catalog_read_names_artists_by_their_whole_credits_and_is_tried_again()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), catalog_credited()).await?;
    *apple.fake.respond_path.lock().unwrap() =
        Some(("/v1/catalog/it/songs".into(), 500, "{}".into()));
    // The list still answers, every credit an artist of its own, whole.
    let by_credits = [
        "Ann & Bob",
        "Artist One",
        "Artist Two",
        "Hiroyuki Sawano",
        "Sawano Hiroyuki & Ann",
        "サワノヒロユキ",
        "サワノヒロユキ & Cara",
    ];
    assert_eq!(names(&all_artists(&apple).await?), by_credits);
    // Once Apple answers again, a later listing asks it behind the list, and the list follows.
    // Apple's answer is held until that listing was checked, so it cannot change the list first.
    *apple.fake.respond_path.lock().unwrap() = None;
    let release = Arc::new(tokio::sync::Notify::new());
    *apple.fake.hold_once.lock().unwrap() = Some(("/v1/catalog/it/songs".into(), release.clone()));
    apple
        .clock
        .advance(crate::apple_music_binding::SHORT_LIST_AGE);
    assert_eq!(names(&all_artists(&apple).await?), by_credits);
    release.notify_one();
    wait_until("the artists to be told apart by the catalog", || async {
        let listed = all_artists(&apple).await?;
        Ok((names(&listed)
            == [
                "Ann",
                "Artist One",
                "Artist Two",
                "Bob",
                "Hiroyuki Sawano",
                "サワノヒロユキ & Cara",
            ])
        .then_some(()))
    })
    .await?;
    apple.finish().await
}

/// The song of the catalog the fake has by id 777, as its menu reads it: from the catalog, the
/// first time.
async fn catalog_song(apple: &OpenApple) -> Result<crate::messages::FetchParsedMediaFileResponse> {
    let id = apple.binding.intern(AppleIdKind::CatalogSong, "777")?;
    apple
        .run(&FetchParsedMediaFileRequest { call_id: None, id })
        .await
}

/// What the UI is told about the sign-in of the Apple Music record, in order.
fn announced_sign_ins(heard: &Heard, record: LibraryRecordId) -> Vec<Option<bool>> {
    heard
        .placed::<crate::messages::LibraryRecordsChanged>("LibraryRecordsChanged")
        .into_iter()
        .filter_map(|(_, changed)| {
            changed
                .records
                .into_iter()
                .find(|summary| summary.record_id == record.0.to_string())
                .map(|summary| summary.signed_in)
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sign_out_while_a_sign_in_is_checked_stands() -> Result<()> {
    use crate::messages::{
        ServiceSignIn, SignInServiceLibraryRequest, SignOutServiceLibraryRequest,
    };
    use crate::utils::{sign_in_service_library_on_node, sign_out_service_library_on_node};

    const CHECK: &str = "/v1/me/storefront";
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let record_id = apple.record.0.to_string();
    let sign_in = || SignInServiceLibraryRequest {
        record_id: record_id.clone(),
        sign_in: ServiceSignIn::Platform {
            user_token: APPLE_OTHER_TOKEN.into(),
            developer_token: Some(DEVELOPER_TOKEN.into()),
        },
        call_id: None,
    };
    // Apple holds its answer to the check of the new sign-in until the sign-out is done.
    let release = Arc::new(tokio::sync::Notify::new());
    *apple.fake.hold_once.lock().unwrap() = Some((CHECK.into(), release.clone()));
    let checks = apple.fake.count(CHECK);
    let signing_in = sign_in();
    let signed_in = sign_in_service_library_on_node(&signing_in, &apple.node);
    let sign_out = async {
        fake_asked(&apple, CHECK, checks + 1).await?;
        sign_out_service_library_on_node(
            &SignOutServiceLibraryRequest {
                record_id: record_id.clone(),
                call_id: None,
            },
            &apple.node,
        )
        .await?;
        release.notify_one();
        Ok::<_, anyhow::Error>(())
    };
    let (signed_in, signed_out) = tokio::join!(signed_in, sign_out);
    signed_out?;
    assert!(signed_in.is_err());
    assert!(!apple.binding.is_signed_in());
    // A sign-in begun after it signs in.
    sign_in_service_library_on_node(&sign_in(), &apple.node).await?;
    assert!(apple.binding.is_signed_in());
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sign_out_while_the_library_is_registered_again_stands() -> Result<()> {
    use crate::messages::{
        RegisterServiceLibraryRequest, ServiceSignIn, SignOutServiceLibraryRequest,
        StreamingService,
    };
    use crate::utils::{register_service_library_on_node, sign_out_service_library_on_node};

    const CHECK: &str = "/v1/me/storefront";
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let register = RegisterServiceLibraryRequest {
        service: StreamingService::AppleMusic,
        alias: "Apple Music".into(),
        sign_in: ServiceSignIn::Platform {
            user_token: APPLE_OTHER_TOKEN.into(),
            developer_token: Some(DEVELOPER_TOKEN.into()),
        },
        call_id: None,
    };
    // Apple holds its answer to the check of the sign-in until the sign-out is done.
    let release = Arc::new(tokio::sync::Notify::new());
    *apple.fake.hold_once.lock().unwrap() = Some((CHECK.into(), release.clone()));
    let checks = apple.fake.count(CHECK);
    let registered = register_service_library_on_node(&register, &apple.node);
    let sign_out = async {
        fake_asked(&apple, CHECK, checks + 1).await?;
        sign_out_service_library_on_node(
            &SignOutServiceLibraryRequest {
                record_id: apple.record.0.to_string(),
                call_id: None,
            },
            &apple.node,
        )
        .await?;
        release.notify_one();
        Ok::<_, anyhow::Error>(())
    };
    let (registered, signed_out) = tokio::join!(registered, sign_out);
    signed_out?;
    assert!(registered.is_err());
    assert!(!apple.binding.is_signed_in());
    // Registered again after it, the library signs in.
    register_service_library_on_node(&register, &apple.node).await?;
    assert!(apple.binding.is_signed_in());
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_ui_is_told_when_an_apple_library_signs_out_and_in_again() -> Result<()> {
    use crate::messages::{
        ListLibraryRecordsRequest, ServiceSignIn, SignInServiceLibraryRequest,
        SignOutServiceLibraryRequest,
    };
    use crate::utils::{sign_in_service_library_on_node, sign_out_service_library_on_node};

    let dir = tempfile::tempdir()?;
    let (apple, heard) = open_apple_heard(dir.path()).await?;
    let node = &apple.node;
    let list = || async {
        ListLibraryRecordsRequest { call_id: None }
            .handle(
                node.clone(),
                None,
                &ListLibraryRecordsRequest { call_id: None },
            )
            .await?
            .context("list library records")
    };
    let sign_state = |listed: &crate::messages::ListLibraryRecordsResponse| {
        listed
            .records
            .iter()
            .find(|summary| summary.record_id == apple.record.0.to_string())
            .map(|summary| summary.signed_in)
    };

    // Signed in by the registration.
    let listed = list().await?;
    assert_eq!(sign_state(&listed), Some(Some(true)));
    // Records with no sign-in of their own say nothing.
    assert!(
        listed
            .records
            .iter()
            .filter(|summary| summary.record_id != apple.record.0.to_string())
            .all(|summary| summary.signed_in.is_none())
    );

    let sign_out = SignOutServiceLibraryRequest {
        record_id: apple.record.0.to_string(),
        call_id: None,
    };
    sign_out_service_library_on_node(&sign_out, node).await?;
    assert!(!apple.binding.is_signed_in());
    assert_eq!(sign_state(&list().await?), Some(Some(false)));
    assert_eq!(
        announced_sign_ins(&heard, apple.record).last(),
        Some(&Some(false))
    );
    // Signing out again is no failure.
    sign_out_service_library_on_node(&sign_out, node).await?;

    sign_in_service_library_on_node(
        &SignInServiceLibraryRequest {
            record_id: apple.record.0.to_string(),
            sign_in: ServiceSignIn::Platform {
                user_token: USER_TOKEN.into(),
                developer_token: Some(DEVELOPER_TOKEN.into()),
            },
            call_id: None,
        },
        node,
    )
    .await?;
    assert_eq!(sign_state(&list().await?), Some(Some(true)));
    assert_eq!(
        announced_sign_ins(&heard, apple.record).last(),
        Some(&Some(true))
    );
    apple.finish().await
}

// Telling the UI ----------------------------------------------------------------------------

/// Waits for the UI to be told that the library changed since it was told `told` times.
async fn told_after(heard: &Heard, told: usize) -> Result<()> {
    wait_until("the UI to be told the library changed", || async {
        Ok((heard.count(CONTENT_CHANGED) > told).then_some(()))
    })
    .await
}

/// Waits until the UI has not been told anything for longer than the binding waits before it
/// tells, and answers how often it was told.
async fn told_settled(heard: &Heard) -> usize {
    loop {
        let told = heard.count(CONTENT_CHANGED);
        tokio::time::sleep(CONTENT_CHANGED_DELAY * 2).await;
        if heard.count(CONTENT_CHANGED) == told {
            return told;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_gone_song_is_left_out_of_the_songs_fetched_by_id() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let first = apple.binding.intern(AppleIdKind::LibrarySong, "i.s9")?;
    let gone = apple
        .binding
        .intern(AppleIdKind::LibrarySong, "catalog:404404")?;
    let last = apple.binding.intern(AppleIdKind::CatalogSong, "777")?;
    let fetched = apple
        .run(&FetchMediaFileByIdsRequest {
            call_id: None,
            ids: vec![first, gone, last],
            bake_cover_arts: false,
        })
        .await?;
    assert_eq!(
        fetched
            .media_files
            .iter()
            .map(|file| file.track.as_str())
            .collect::<Vec<_>>(),
        ["i.s9", "catalog:777"]
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_artist_the_library_no_longer_lists_is_skipped_by_the_collections_fetched_by_id()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    // Ids of before the list told its artists apart, which name artists by their spelling.
    let gone = apple
        .binding
        .intern(AppleIdKind::Artist, "name:nobody of this name")?;
    let held = apple
        .binding
        .intern(AppleIdKind::Artist, "name:artist one")?;
    let fetched = apple
        .run(&FetchCollectionByIdsRequest {
            call_id: None,
            collection_type: CollectionType::Artist,
            bake_cover_arts: false,
            ids: vec![gone, held],
        })
        .await?;
    assert_eq!(
        fetched
            .result
            .iter()
            .map(|artist| (artist.id, artist.name.as_str()))
            .collect::<Vec<_>>(),
        [(held, "Artist One")]
    );
    apple.finish().await
}

// Rune's own playlists ----------------------------------------------------------------------

/// Waits for the changes to come to rest, and fails unless the UI was told of them `times` more
/// times than `told`; answers how often it was told.
async fn told_more(heard: &Heard, told: usize, times: usize, change: &str) -> Result<usize> {
    let now = told_settled(heard).await;
    anyhow::ensure!(
        now == told + times,
        "{change}: told {} times, not {times}",
        now.saturating_sub(told)
    );
    Ok(now)
}

/// Where the Rune playlists of the open Apple library are kept.
fn playlists_file(apple: &OpenApple) -> std::path::PathBuf {
    use crate::service::device_records::device_records_folder;
    use crate::service::rune_playlists::PLAYLISTS_FILE;

    device_records_folder(Path::new(&*apple.node.config_path), apple.record.0).join(PLAYLISTS_FILE)
}

/// Makes a Rune playlist in the open Apple library.
async fn make_rune_playlist(
    apple: &OpenApple,
    name: &str,
    group: &str,
) -> Result<crate::messages::Playlist> {
    Ok(apple
        .run(&CreatePlaylistRequest {
            name: name.into(),
            group: group.into(),
            call_id: None,
        })
        .await?
        .playlist)
}

/// Adds the song `track` to a playlist at `position`, and answers its occurrence id.
async fn add_song(
    apple: &OpenApple,
    playlist_id: i32,
    track: &str,
    position: Option<i32>,
) -> Result<String> {
    Ok(apple
        .run(&AddItemToPlaylistRequest {
            playlist_id,
            media_file_id: apple.binding.intern_track(track)?,
            position,
            call_id: None,
        })
        .await?
        .occurrence_id)
}

async fn playlist_named(apple: &OpenApple, name: &str) -> Result<i32> {
    apple
        .run(&FetchAllPlaylistsRequest { call_id: None })
        .await?
        .playlists
        .into_iter()
        .find(|playlist| playlist.name == name)
        .map(|playlist| playlist.id)
        .with_context(|| format!("the playlist {name}"))
}

fn playlist_page(playlist_id: i32) -> FetchPlaylistOccurrencesRequest {
    FetchPlaylistOccurrencesRequest {
        request_id: "page".into(),
        playlist_id,
        after: None,
        page_size: 30,
        bake_cover_arts: false,
        call_id: None,
    }
}

/// The tracks of a playlist page and their occurrence ids, in order.
fn occurrences(page: &crate::messages::FetchPlaylistOccurrencesResponse) -> Vec<(&str, &str)> {
    page.entries
        .iter()
        .map(|entry| (entry.track_uuid.as_str(), entry.occurrence_id.as_str()))
        .collect()
}

/// The tracks of a mix page, in order.
fn mix_tracks(page: crate::messages::MixQueryResponse) -> Vec<String> {
    page.files.into_iter().map(|file| file.track).collect()
}

fn invalid_input(error: &anyhow::Error) -> bool {
    classify(error) == crate::messages::RequestErrorKind::InvalidInput
}

fn not_found(error: &anyhow::Error) -> bool {
    classify(error) == crate::messages::RequestErrorKind::NotFound
}

/// The names and groups of the playlists songs can be added to, in order.
async fn offered_playlists(apple: &OpenApple) -> Result<Vec<(String, String)>> {
    Ok(apple
        .run(&FetchAllPlaylistsRequest { call_id: None })
        .await?
        .playlists
        .into_iter()
        .map(|playlist| (playlist.name, playlist.group))
        .collect())
}

/// The groups of the Playlists page, with how many playlists each holds.
async fn playlist_groups(apple: &OpenApple) -> Result<Vec<(String, i32)>> {
    Ok(apple
        .run(&FetchCollectionGroupSummaryRequest {
            call_id: None,
            collection_type: CollectionType::Playlist,
        })
        .await?
        .groups
        .into_iter()
        .map(|group| (group.group_title, group.count))
        .collect())
}

/// The home's Playlists group, as names and whether each can be changed.
async fn home_playlists(apple: &OpenApple) -> Result<Vec<crate::messages::ComplexQueryEntry>> {
    let home = apple
        .run(&ComplexQueryRequest {
            queries: vec![ComplexQuery {
                title: "Playlists".into(),
                query: LibraryHomeQuery::Playlists(LibraryHomeListMode::Forward),
            }],
            call_id: None,
        })
        .await?;
    Ok(home.result.into_iter().next().context("the group")?.entries)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_playlist_of_apples_is_listed_or_written_to() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            playlists: true,
            ..AppleOptions::default()
        },
    )
    .await?;
    // Apple's own playlists are listed only as the Rune playlists the sign-in imported them
    // into, in Apple's order, in the group the library fills itself.
    let imported: Vec<(String, String)> = ["Road Trip", "Sunday", "Favourite Songs", "Evening Jazz"]
        .iter()
        .map(|name| ((*name).to_owned(), "Apple Music".to_owned()))
        .collect();
    assert_eq!(offered_playlists(&apple).await?, imported);
    let summary = apple
        .run(&FetchCollectionGroupSummaryRequest {
            call_id: None,
            collection_type: CollectionType::Playlist,
        })
        .await?;
    assert_eq!(
        summary
            .groups
            .iter()
            .map(|group| (group.group_title.as_str(), group.count))
            .collect::<Vec<_>>(),
        [("Apple Music", 4)]
    );
    assert_eq!(summary.reserved_groups, ["Apple Music"]);
    assert!(home_playlists(&apple).await?.iter().all(|entry| !entry.readonly));

    // No id names one of Apple's, and a request that names an id nothing was given for is
    // refused.
    let unknown = 4242;
    let refused = [
        apple
            .run(&GetPlaylistByIdRequest {
                playlist_id: unknown,
                call_id: None,
            })
            .await
            .err(),
        apple.run(&playlist_page(unknown)).await.err(),
        add_song(&apple, unknown, "i.s2", None).await.err(),
        apple
            .run(&UpdatePlaylistRequest {
                playlist_id: unknown,
                name: "Renamed".into(),
                group: String::new(),
                call_id: None,
            })
            .await
            .err(),
        apple
            .run(&RemovePlaylistRequest {
                playlist_id: unknown,
                call_id: None,
            })
            .await
            .err(),
        apple
            .run(&ReorderPlaylistItemPositionRequest {
                playlist_id: unknown,
                occurrence_id: "0".into(),
                new_position: 1,
                call_id: None,
            })
            .await
            .err(),
        apple
            .run(&RemoveItemFromPlaylistRequest {
                playlist_id: unknown,
                occurrence_id: "0".into(),
                call_id: None,
            })
            .await
            .err(),
    ];
    for error in refused {
        let error = error.context("a refused playlist request")?;
        assert!(invalid_input(&error), "{error:#}");
    }

    // A playlist of Rune's is made, filled and changed on this device only.
    let mine = make_rune_playlist(&apple, "Mine", "Mood").await?;
    let first = add_song(&apple, mine.id, "i.s1", None).await?;
    add_song(&apple, mine.id, "i.s2", Some(0)).await?;
    apple
        .run(&ReorderPlaylistItemPositionRequest {
            playlist_id: mine.id,
            occurrence_id: first,
            new_position: 0,
            call_id: None,
        })
        .await?;
    let mut offered = imported;
    offered.push(("Mine".to_owned(), "Mood".to_owned()));
    assert_eq!(offered_playlists(&apple).await?, offered);
    assert!(apple.fake.writes.lock().unwrap().is_empty());
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_rune_playlist_takes_songs_anywhere_and_keeps_their_occurrences_as_they_move()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let (apple, heard) = open_apple_heard(dir.path()).await?;
    // Every change tells the UI once; a refused one does not.
    let mut told = told_settled(&heard).await;
    let night = make_rune_playlist(&apple, "Night Drive", "Mood").await?;
    told = told_more(&heard, told, 1, "create").await?;
    assert_eq!(
        (night.name.as_str(), night.group.as_str()),
        ("Night Drive", "Mood")
    );

    let first = add_song(&apple, night.id, "i.s1", None).await?;
    told = told_more(&heard, told, 1, "add at the end").await?;
    let third = add_song(&apple, night.id, "i.s3", Some(0)).await?;
    told = told_more(&heard, told, 1, "add first").await?;
    // A catalog song is kept as it is: nothing is added to the library.
    let catalog = add_song(&apple, night.id, "catalog:777", Some(1)).await?;
    told = told_more(&heard, told, 1, "add a catalog song").await?;
    let second = add_song(&apple, night.id, "i.s2", Some(99)).await?;
    told = told_more(&heard, told, 1, "add past the end").await?;
    let mut page = playlist_page(night.id);
    let all = apple.run(&page).await?;
    assert_eq!(
        occurrences(&all),
        [
            ("i.s3", third.as_str()),
            ("catalog:777", catalog.as_str()),
            ("i.s1", first.as_str()),
            ("i.s2", second.as_str()),
        ]
    );
    assert_eq!(
        all.entries
            .iter()
            .map(|entry| (
                entry.position,
                entry.file.as_ref().map(|file| file.title.as_str())
            ))
            .collect::<Vec<_>>(),
        [
            (0, Some("Third Road")),
            (1, Some("Catalog Tune")),
            (2, Some("First Light")),
            (3, Some("Second Wind")),
        ]
    );
    // Pages follow each other by the entry the last one ended with.
    page.page_size = 3;
    let head = apple.run(&page).await?;
    assert_eq!(head.entries.len(), 3);
    page.after = head.next.clone();
    let tail = apple.run(&page).await?;
    assert_eq!(occurrences(&tail), [("i.s2", second.as_str())]);
    assert!(tail.next.is_none());
    // A page larger than a listing takes is refused.
    page.page_size = 129;
    let too_large = apple.run(&page).await.err().context("a page too large")?;
    assert!(invalid_input(&too_large), "{too_large:#}");

    apple
        .run(&ReorderPlaylistItemPositionRequest {
            playlist_id: night.id,
            occurrence_id: second.clone(),
            new_position: 0,
            call_id: None,
        })
        .await?;
    told = told_more(&heard, told, 1, "reorder").await?;
    let remove = RemoveItemFromPlaylistRequest {
        playlist_id: night.id,
        occurrence_id: catalog.clone(),
        call_id: None,
    };
    apple.run(&remove).await?;
    told = told_more(&heard, told, 1, "remove").await?;
    let after = apple.run(&playlist_page(night.id)).await?;
    assert_eq!(
        occurrences(&after),
        [
            ("i.s2", second.as_str()),
            ("i.s3", third.as_str()),
            ("i.s1", first.as_str()),
        ]
    );
    // An entry already taken out is not found again, and a page that ended with it goes on
    // from where it was.
    let again = apple.run(&remove).await.err().context("a removed entry")?;
    assert!(not_found(&again), "{again:#}");
    told = told_more(&heard, told, 0, "a refused removal").await?;
    let mut from_removed = playlist_page(night.id);
    from_removed.after = Some(crate::messages::PlaylistOccurrenceCursor {
        position: 1,
        occurrence_id: catalog,
    });
    assert_eq!(
        occurrences(&apple.run(&from_removed).await?),
        [("i.s3", third.as_str()), ("i.s1", first.as_str())]
    );
    let before = add_song(&apple, night.id, "i.s1", Some(-1))
        .await
        .err()
        .context("a negative position")?;
    assert!(invalid_input(&before), "{before:#}");
    told_more(&heard, told, 0, "a refused position").await?;

    // Nothing reached Apple.
    assert!(apple.fake.writes.lock().unwrap().is_empty());
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_rune_playlist_is_renamed_regrouped_and_removed_but_not_put_in_apples_group() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let night = make_rune_playlist(&apple, "Night Drive", "Mood").await?;
    let update = |name: &str, group: &str| UpdatePlaylistRequest {
        playlist_id: night.id,
        name: name.into(),
        group: group.into(),
        call_id: None,
    };
    // A group is kept as it is given, and may be empty, as in a local library.
    let ungrouped = apple.run(&update("Night Drive", "")).await?.playlist;
    assert_eq!(ungrouped.group, "");
    let renamed = apple.run(&update("Late Drive", "Calm")).await?.playlist;
    assert_eq!(
        (renamed.id, renamed.name.as_str(), renamed.group.as_str()),
        (night.id, "Late Drive", "Calm")
    );
    let opened = apple
        .run(&GetPlaylistByIdRequest {
            playlist_id: night.id,
            call_id: None,
        })
        .await?
        .playlist;
    assert_eq!(
        (opened.name.as_str(), opened.group.as_str()),
        ("Late Drive", "Calm")
    );

    // The group of Apple Music's playlists is filled by the library itself.
    for group in ["Apple Music", " apple music "] {
        let made = make_rune_playlist(&apple, "Other", group)
            .await
            .err()
            .context("a Rune playlist in Apple's group")?;
        assert!(invalid_input(&made), "{made:#}");
        let moved = apple
            .run(&update("Late Drive", group))
            .await
            .err()
            .context("a Rune playlist moved to Apple's group")?;
        assert!(invalid_input(&moved), "{moved:#}");
    }

    let removed = apple
        .run(&RemovePlaylistRequest {
            playlist_id: night.id,
            call_id: None,
        })
        .await?;
    assert_eq!(removed.playlist_id, night.id);
    let gone = apple
        .run(&GetPlaylistByIdRequest {
            playlist_id: night.id,
            call_id: None,
        })
        .await
        .err()
        .context("a removed playlist")?;
    assert!(not_found(&gone), "{gone:#}");
    let renamed_gone = apple
        .run(&update("Later", "Calm"))
        .await
        .err()
        .context("a removed playlist renamed")?;
    assert!(not_found(&renamed_gone), "{renamed_gone:#}");
    assert!(offered_playlists(&apple).await?.is_empty());
    // A page that still holds its id leaves it out, as a local library does.
    let by_ids = apple
        .run(&FetchCollectionByIdsRequest {
            call_id: None,
            collection_type: CollectionType::Playlist,
            bake_cover_arts: false,
            ids: vec![night.id],
        })
        .await?;
    assert!(by_ids.result.is_empty());
    assert!(apple.fake.writes.lock().unwrap().is_empty());
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rune_playlists_are_listed_opened_and_played() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let night = make_rune_playlist(&apple, "Night Drive", "Mood").await?;
    add_song(&apple, night.id, "i.s3", None).await?;
    add_song(&apple, night.id, "i.s1", None).await?;
    make_rune_playlist(&apple, "Morning", "Calm").await?;

    // Songs can be added to Rune's playlists, by group.
    assert_eq!(
        offered_playlists(&apple).await?,
        [
            ("Morning".to_owned(), "Calm".to_owned()),
            ("Night Drive".to_owned(), "Mood".to_owned()),
        ]
    );
    assert_eq!(
        playlist_groups(&apple).await?,
        [("Calm".to_owned(), 1), ("Mood".to_owned(), 1)]
    );
    let groups = apple
        .run(&FetchCollectionGroupsRequest {
            call_id: None,
            collection_type: CollectionType::Playlist,
            bake_cover_arts: false,
            group_titles: vec!["Calm".into(), "Mood".into()],
        })
        .await?;
    let shown = |index: usize| {
        groups.groups[index]
            .collections
            .iter()
            .map(|collection| (collection.name.as_str(), collection.readonly))
            .collect::<Vec<_>>()
    };
    // Rune's can be changed here.
    assert_eq!(shown(0), [("Morning", false)]);
    assert_eq!(shown(1), [("Night Drive", false)]);
    // Its cover is that of its first song, which the binding holds already.
    wait_until("the cover of the Rune playlist", || async {
        let by_ids = apple
            .run(&FetchCollectionByIdsRequest {
                call_id: None,
                collection_type: CollectionType::Playlist,
                bake_cover_arts: true,
                ids: vec![night.id],
            })
            .await?;
        assert_eq!(by_ids.result[0].name, "Night Drive");
        assert!(!by_ids.result[0].readonly);
        Ok((!by_ids.result[0].cover_art_map.is_empty()).then_some(()))
    })
    .await?;
    assert!(apple.fake.count("/art/Night_Album/") > 0);
    let searched = apple
        .run(&SearchCollectionSummaryRequest {
            call_id: None,
            collection_type: Some(CollectionType::Playlist),
            bake_cover_arts: Some(false),
            n: 1,
        })
        .await?;
    assert_eq!(
        searched
            .result
            .iter()
            .map(|playlist| playlist.name.as_str())
            .collect::<Vec<_>>(),
        ["Night Drive"]
    );

    let home = home_playlists(&apple).await?;
    assert_eq!(
        home.iter()
            .map(|entry| (entry.name.as_str(), entry.readonly))
            .collect::<Vec<_>>(),
        [("Night Drive", false), ("Morning", false)]
    );
    let entry = &home[0];
    assert_eq!(entry.queries.len(), 1);
    assert_eq!(
        (
            entry.queries[0].operator.as_str(),
            entry.queries[0].parameter.clone()
        ),
        ("lib::playlist", night.id.to_string())
    );

    let resolved = apple
        .run(&ResolveMixQueryRequest {
            queries: entry.queries.clone(),
            hint_position: 1,
            initial_playback_item: Some(song_ref(&apple, "i.s1")),
            instantly_play: true,
            fallback_playing_items: Vec::new(),
            excluded: ExcludedItems::default(),
            call_id: None,
        })
        .await?;
    assert_eq!(
        resolved.playing_items,
        [song_ref(&apple, "i.s3"), song_ref(&apple, "i.s1")]
    );
    assert_eq!(resolved.start_index, Some(1));
    assert_eq!(
        mix_tracks(apple.run(&mix("lib::playlist", night.id)).await?),
        ["i.s3", "i.s1"]
    );
    // Together with a song, as a mix of several items plays them.
    let mut together = mix("lib::playlist", night.id);
    together.queries.push(MixQuery {
        operator: "lib::track".into(),
        parameter: apple.binding.intern_track("i.s2")?.to_string(),
    });
    assert_eq!(
        mix_tracks(apple.run(&together).await?),
        ["i.s2", "i.s3", "i.s1"]
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rune_playlists_last_through_sign_ins_and_new_bindings_and_outlive_the_record() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let file = playlists_file(&apple);
    let night = make_rune_playlist(&apple, "Night Drive", "Mood").await?;
    let occurrence = add_song(&apple, night.id, "i.s2", None).await?;
    assert!(file.exists());
    let tracks_of = |page: crate::messages::FetchPlaylistOccurrencesResponse| {
        page.entries
            .into_iter()
            .map(|entry| (entry.track_uuid, entry.occurrence_id))
            .collect::<Vec<_>>()
    };
    let kept = [("i.s2".to_owned(), occurrence.clone())];

    // Signing out drops what was read from Apple, not Rune's playlists or their ids.
    apple.binding.sign_out();
    apple
        .binding
        .sign_in(Tokens::new(DEVELOPER_TOKEN, APPLE_OTHER_TOKEN).context("tokens")?);
    assert_eq!(playlist_named(&apple, "Night Drive").await?, night.id);
    assert_eq!(tracks_of(apple.run(&playlist_page(night.id)).await?), kept);

    // A binding made afresh, as after a restart, reads them from the file before its sign-in.
    let restarted = restart_apple(apple).await?;
    let (ui, binding) = restarted.open().await?;
    let id = dispatch_gui_request(&ui, &FetchAllPlaylistsRequest { call_id: None })
        .await?
        .context("the playlists")?
        .playlists
        .into_iter()
        .find(|playlist| playlist.name == "Night Drive")
        .context("the playlist")?
        .id;
    let page = |id| {
        let ui = &ui;
        async move {
            dispatch_gui_request(ui, &playlist_page(id))
                .await?
                .context("the playlist page")
        }
    };
    assert_eq!(tracks_of(page(id).await?), kept);
    restarted.sign_in(USER_TOKEN).await?;
    let signed_in = page(id).await?;
    assert_eq!(
        signed_in.entries[0]
            .file
            .as_ref()
            .map(|file| file.title.as_str()),
        Some("Second Wind")
    );

    // Removing the record keeps them on this device, and its binding writes nothing any more.
    let registry = LibraryRegistry::load(Path::new(&*restarted.node.config_path))?;
    let own = registry
        .libraries
        .iter()
        .find(|record| matches!(record.protocol, ProtocolAdapter::Native))
        .context("the fixture's own library")?;
    super::open_registered_library_record(restarted.node.clone(), own.record_id.0.to_string())
        .await?;
    let written = std::fs::read(&file)?;
    let removal = crate::messages::RemoveLibraryRecordRequest {
        record_id: restarted.record.0.to_string(),
        call_id: None,
        delete_device_data: false,
    };
    <crate::messages::RemoveLibraryRecordRequest as crate::Signal>::handle(
        &removal,
        restarted.node.clone(),
        None,
        &removal,
    )
    .await?
    .context("the record's removal")?;
    let refused = binding
        .rune_playlists
        .create("Late", "Mood")
        .await
        .err()
        .context("a change after the record is removed")?;
    assert!(matches!(
        refused.downcast_ref::<ExpectedRequestError>(),
        Some(ExpectedRequestError::Unavailable { .. })
    ));
    assert_eq!(std::fs::read(&file)?, written);
    restarted.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn songs_apple_no_longer_has_are_asked_for_once_and_mix_pages_still_fill() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let night = make_rune_playlist(&apple, "Night Drive", "Mood").await?;
    for track in ["i.gone", "i.s1", "i.lost", "i.s3", "i.s2"] {
        add_song(&apple, night.id, track, None).await?;
    }
    let asked_for = |track: &str| {
        apple
            .fake
            .seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(target, _)| {
                target.starts_with("/v1/me/library/songs?") && target.contains(track)
            })
            .count()
    };
    let page = apple.run(&playlist_page(night.id)).await?;
    assert_eq!(
        page.entries
            .iter()
            .map(|entry| (
                entry.track_uuid.as_str(),
                entry.file.as_ref().map(|file| file.title.as_str())
            ))
            .collect::<Vec<_>>(),
        [
            ("i.gone", None),
            ("i.s1", Some("First Light")),
            ("i.lost", None),
            ("i.s3", Some("Third Road")),
            ("i.s2", Some("Second Wind")),
        ]
    );
    let (gone, lost) = (asked_for("i.gone"), asked_for("i.lost"));
    assert!(gone > 0 && lost > 0);
    // Listed again, the songs Apple said it does not have are not asked for again.
    apple.run(&playlist_page(night.id)).await?;
    assert_eq!((asked_for("i.gone"), asked_for("i.lost")), (gone, lost));

    // A mix query pages through the songs Apple has, a full page at a time.
    let mut query = mix("lib::playlist", night.id);
    query.page_size = 2;
    assert_eq!(mix_tracks(apple.run(&query).await?), ["i.s1", "i.s3"]);
    query.cursor = 2;
    assert_eq!(mix_tracks(apple.run(&query).await?), ["i.s2"]);
    query.cursor = 3;
    assert!(mix_tracks(apple.run(&query).await?).is_empty());
    query.cursor = 0;
    query.page_size = 3;
    assert_eq!(
        mix_tracks(apple.run(&query).await?),
        ["i.s1", "i.s3", "i.s2"]
    );
    assert_eq!((asked_for("i.gone"), asked_for("i.lost")), (gone, lost));

    // Another sign-in may have them, so it asks again.
    apple
        .binding
        .sign_in(Tokens::new(DEVELOPER_TOKEN, APPLE_OTHER_TOKEN).context("tokens")?);
    apple.run(&playlist_page(night.id)).await?;
    assert!(asked_for("i.gone") > gone);
    apple.finish().await
}

/// How often Apple was asked for library songs by id, in all and for `track`.
fn asked_by_id(apple: &OpenApple, track: &str) -> (usize, usize) {
    let seen = apple.fake.seen.lock().unwrap();
    let lookups: Vec<&String> = seen
        .iter()
        .map(|(target, _)| target)
        .filter(|target| target.starts_with("/v1/me/library/songs?") && target.contains("ids="))
        .collect();
    let of_track = lookups
        .iter()
        .filter(|target| target.contains(track))
        .count();
    (lookups.len(), of_track)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mix_pages_of_a_long_rune_playlist_fill_across_its_lookup_steps() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_big_apple(dir.path(), 70).await?;
    // 72 entries, looked up 64 at a time: one gone song in each step.
    let made = apple
        .binding
        .rune_playlists
        .create("Long Drive", "Mood")
        .await?;
    let mut extras = (0..70).map(|n| format!("i.x{n:04}"));
    for index in 0..72 {
        let track = match index {
            10 => "i.gone1".to_owned(),
            66 => "i.gone2".to_owned(),
            _ => extras.next().context("an extra song")?,
        };
        apple
            .binding
            .rune_playlists
            .add(&made.id, &track, Default::default(), None)
            .await?;
    }
    let long = playlist_named(&apple, "Long Drive").await?;
    let page = |cursor: i32, page_size: i32| {
        let mut query = mix("lib::playlist", long);
        query.cursor = cursor;
        query.page_size = page_size;
        query
    };
    let songs =
        |from: usize, to: usize| (from..to).map(|n| format!("i.x{n:04}")).collect::<Vec<_>>();
    assert_eq!(mix_tracks(apple.run(&page(0, 30)).await?), songs(0, 30));
    // These need the songs of the second step.
    assert_eq!(mix_tracks(apple.run(&page(62, 3)).await?), songs(62, 65));
    assert_eq!(mix_tracks(apple.run(&page(60, 30)).await?), songs(60, 70));
    assert!(mix_tracks(apple.run(&page(70, 30)).await?).is_empty());
    // Every page looked the songs before it up again, but Apple was asked for each gone song
    // once, in the lookup of its step, and for no song of the list at all.
    assert_eq!(asked_by_id(&apple, "i.gone1"), (2, 1));
    assert_eq!(asked_by_id(&apple, "i.gone2"), (2, 1));
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_song_taken_for_missing_is_found_again_once_it_is_listed() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let night = make_rune_playlist(&apple, "Night Drive", "Mood").await?;
    add_song(&apple, night.id, "i.s1", None).await?;
    let titles = || async {
        Ok::<_, anyhow::Error>(
            apple
                .run(&playlist_page(night.id))
                .await?
                .entries
                .into_iter()
                .map(|entry| entry.file.map(|file| file.title))
                .collect::<Vec<_>>(),
        )
    };
    // Apple's song list holds it, so a page finds it though it was taken for missing.
    apple
        .binding
        .remember_missing(apple.binding.generation(), "i.s1");
    assert_eq!(titles().await?, [Some("First Light".to_owned())]);
    // A song the list lacks is found again once the UI lists it to be added.
    apple
        .binding
        .remember_missing(apple.binding.generation(), "i.s9");
    add_song(&apple, night.id, "i.s9", None).await?;
    assert_eq!(
        titles().await?,
        [Some("First Light".to_owned()), Some("Ninth Moon".to_owned())]
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_entry_whose_song_is_gone_plays_its_catalog_copy_or_keeps_its_name() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let night = make_rune_playlist(&apple, "Night Drive", "Mood").await?;
    // Two songs the UI lists before they are added, which Apple no longer has later.
    apple.fake.old_songs.store(true, Ordering::SeqCst);
    let listed = apple
        .run(&FetchMediaFileByIdsRequest {
            call_id: None,
            ids: vec![
                apple.binding.intern_track("i.copy")?,
                apple.binding.intern_track("i.lost")?,
            ],
            bake_cover_arts: false,
        })
        .await?;
    assert_eq!(listed.media_files.len(), 2);
    let copy = add_song(&apple, night.id, "i.copy", None).await?;
    let lost = add_song(&apple, night.id, "i.lost", None).await?;
    let file = std::fs::read_to_string(playlists_file(&apple))?;
    assert!(file.contains("\"catalog_id\": \"777\"") && file.contains("Lost Song"));

    // Signed out and in again, the binding holds neither, and Apple has neither any more.
    apple.fake.old_songs.store(false, Ordering::SeqCst);
    apple.binding.sign_out();
    apple
        .binding
        .sign_in(Tokens::new(DEVELOPER_TOKEN, USER_TOKEN).context("tokens")?);
    let page = apple.run(&playlist_page(night.id)).await?;
    let [copied, gone] = &page.entries[..] else {
        anyhow::bail!("two entries, not {}", page.entries.len());
    };
    // The library copy is gone, so the catalog song it was a copy of plays in its place.
    assert_eq!(
        (copied.occurrence_id.as_str(), copied.track_uuid.as_str()),
        (copy.as_str(), "catalog:777")
    );
    let copied = copied.file.as_ref().context("the catalog song")?;
    assert_eq!(
        (copied.title.as_str(), copied.audio_availability),
        ("Catalog Tune", AudioAvailability::RemoteOnly)
    );
    assert!(copied.id > 0);
    // The other keeps the name it had, and cannot play.
    assert_eq!(
        (gone.occurrence_id.as_str(), gone.track_uuid.as_str()),
        (lost.as_str(), "i.lost")
    );
    let gone = gone.file.as_ref().context("the remembered song")?;
    assert_eq!(
        (
            gone.id,
            gone.title.as_str(),
            gone.artist.as_str(),
            gone.album.as_str(),
            gone.audio_availability
        ),
        (
            0,
            "Lost Song",
            "Old Artist",
            "Old Album",
            AudioAvailability::Unavailable
        )
    );

    let resolved = apple
        .run(&ResolveMixQueryRequest {
            queries: vec![MixQuery {
                operator: "lib::playlist".into(),
                parameter: night.id.to_string(),
            }],
            hint_position: 0,
            initial_playback_item: Some(song_ref(&apple, "catalog:777")),
            instantly_play: true,
            fallback_playing_items: Vec::new(),
            excluded: ExcludedItems::default(),
            call_id: None,
        })
        .await?;
    assert_eq!(
        resolved.playing_items,
        [song_ref(&apple, "catalog:777"), song_ref(&apple, "i.lost")]
    );
    assert_eq!(resolved.start_index, Some(0));
    assert_eq!(
        mix_tracks(apple.run(&mix("lib::playlist", night.id)).await?),
        ["catalog:777"]
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_playlist_file_that_cannot_be_read_is_reported_and_left_as_it_is() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    // The file is spoiled, and a binding made afresh, as after a restart, reads it.
    let file = playlists_file(&apple);
    std::fs::write(&file, b"{ not the playlists")?;
    let (_ui, binding) = bind_again(&apple).await?;

    // No playlist is listed, and the lists that show them still answer.
    assert!(offered_playlists(&apple).await?.is_empty());
    assert!(playlist_groups(&apple).await?.is_empty());
    assert!(home_playlists(&apple).await?.is_empty());

    // What uses a Rune playlist fails, naming the file it could not read.
    let names_the_file = |error: Option<anyhow::Error>| {
        let error = format!("{:#}", error.expect("a refusal"));
        assert!(
            error.contains("could not be read") && error.contains(&file.display().to_string()),
            "{error}"
        );
    };
    names_the_file(
        make_rune_playlist(&apple, "Night Drive", "Mood")
            .await
            .err(),
    );
    // An id the UI held from before.
    let held = binding.intern(AppleIdKind::RunePlaylist, "held")?;
    names_the_file(
        apple
            .run(&GetPlaylistByIdRequest {
                playlist_id: held,
                call_id: None,
            })
            .await
            .err(),
    );
    names_the_file(apple.run(&playlist_page(held)).await.err());
    names_the_file(
        apple
            .run(&AddItemToPlaylistRequest {
                playlist_id: held,
                media_file_id: binding.intern_track("i.s1")?,
                position: None,
                call_id: None,
            })
            .await
            .err(),
    );
    assert_eq!(std::fs::read(&file)?, b"{ not the playlists");
    assert!(apple.fake.writes.lock().unwrap().is_empty());

    // Once the file is gone, Rune's playlists are made and listed again.
    std::fs::remove_file(&file)?;
    make_rune_playlist(&apple, "Night Drive", "Mood").await?;
    assert_eq!(
        offered_playlists(&apple).await?,
        [("Night Drive".to_owned(), "Mood".to_owned())]
    );
    apple.finish().await
}

// Mixes: Rune's own and the ones Apple recommends -------------------------------------------

fn recommending() -> AppleOptions {
    AppleOptions {
        recommends: true,
        ..AppleOptions::default()
    }
}

/// The Mixes page as it starts: the title and count of each group.
async fn mix_summary(apple: &OpenApple) -> Result<Vec<(String, i32)>> {
    Ok(apple
        .run(&FetchCollectionGroupSummaryRequest {
            collection_type: CollectionType::Mix,
            call_id: None,
        })
        .await?
        .groups
        .into_iter()
        .map(|group| (group.group_title, group.count))
        .collect())
}

/// The groups of mixes titled `titles`, as the Mixes page loads them.
async fn mix_groups(apple: &OpenApple, titles: &[&str]) -> Result<Vec<CollectionGroup>> {
    Ok(apple
        .run(&FetchCollectionGroupsRequest {
            collection_type: CollectionType::Mix,
            bake_cover_arts: true,
            group_titles: titles.iter().map(|title| (*title).to_owned()).collect(),
            call_id: None,
        })
        .await?
        .groups)
}

fn mix_names(group: &CollectionGroup) -> Vec<&str> {
    group
        .collections
        .iter()
        .map(|collection| collection.name.as_str())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_that_bakes_no_covers_gets_none_though_they_are_cached() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), recommending()).await?;
    let album = album_listed(&apple, "Dawn Album").await?;
    let song = apple.binding.intern_track("i.s1")?;
    // How many covers an album, a song, the recommended mixes and the Tracks page come with.
    let covers = async |bake| -> Result<(usize, usize, usize, usize)> {
        let albums = apple
            .run(&FetchCollectionByIdsRequest {
                call_id: None,
                collection_type: CollectionType::Album,
                bake_cover_arts: bake,
                ids: vec![album],
            })
            .await?;
        let files = apple
            .run(&FetchMediaFileByIdsRequest {
                ids: vec![song],
                bake_cover_arts: bake,
                call_id: None,
            })
            .await?;
        let mixes = apple
            .run(&FetchCollectionGroupsRequest {
                collection_type: CollectionType::Mix,
                bake_cover_arts: bake,
                group_titles: vec!["Made for You".into()],
                call_id: None,
            })
            .await?;
        let tracks = apple
            .run(&MixQueryRequest {
                bake_cover_arts: bake,
                ..mix("lib::all", "true")
            })
            .await?;
        Ok((
            albums.result[0].cover_art_map.len(),
            files.cover_art_map.len(),
            mixes.groups[0]
                .collections
                .iter()
                .map(|mix| mix.cover_art_map.len())
                .sum(),
            tracks.cover_art_map.len(),
        ))
    };
    wait_until("the covers to be cached", || async {
        Ok((covers(true).await? == (1, 1, 3, 3)).then_some(()))
    })
    .await?;
    assert_eq!(covers(false).await?, (0, 0, 0, 0));
    apple.finish().await
}

/// The queries a mix page plays, as it fetches them.
async fn stored_queries(apple: &OpenApple, mix_id: i32) -> Result<Vec<MixQuery>> {
    Ok(apple
        .run(&FetchMixQueriesRequest {
            mix_id,
            call_id: None,
        })
        .await?
        .result
        .into_iter()
        .map(|query| {
            assert!(query.available);
            MixQuery {
                operator: query.operator,
                parameter: query.parameter,
            }
        })
        .collect())
}

fn home_mixes(mode: LibraryHomeListMode) -> ComplexQueryRequest {
    ComplexQueryRequest {
        queries: vec![ComplexQuery {
            title: "Mixes".into(),
            query: LibraryHomeQuery::Mixes(mode),
        }],
        call_id: None,
    }
}

fn entry_names(response: &crate::messages::ComplexQueryResponse) -> Vec<&str> {
    response.result[0]
        .entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect()
}

fn query(operator: &str, parameter: impl ToString) -> MixQuery {
    MixQuery {
        operator: operator.into(),
        parameter: parameter.to_string(),
    }
}

/// Makes a Rune mix of `queries` in the group "Favorite".
async fn create_mix(
    apple: &OpenApple,
    name: &str,
    queries: Vec<MixQuery>,
) -> Result<crate::messages::Mix> {
    Ok(apple
        .run(&crate::messages::CreateMixRequest {
            name: name.into(),
            group: "Favorite".into(),
            mode: None,
            queries,
            call_id: None,
        })
        .await?
        .mix)
}

/// The mixes the menus offer to add to.
async fn rune_mixes(apple: &OpenApple) -> Result<Vec<crate::messages::Mix>> {
    Ok(apple
        .run(&FetchAllMixesRequest { call_id: None })
        .await?
        .mixes)
}

/// The titles of the songs a mix page lists for `queries`.
async fn mix_titles(apple: &OpenApple, queries: Vec<MixQuery>) -> Result<Vec<String>> {
    let mut request = mix("lib::all", "true");
    request.queries = queries;
    Ok(apple
        .run(&request)
        .await?
        .files
        .into_iter()
        .map(|file| file.title)
        .collect())
}

/// What Apple keys the items of a mix's queries by, in order.
fn query_keys(binding: &AppleMusicBinding, queries: &[MixQuery]) -> Result<Vec<String>> {
    queries
        .iter()
        .map(|query| {
            let id = query.parameter.parse()?;
            match query.operator.as_str() {
                "lib::artist" => binding.resolve(AppleIdKind::Artist, id),
                "lib::album" => binding.resolve(AppleIdKind::Album, id),
                "lib::playlist" => binding.resolve(AppleIdKind::Playlist, id),
                "lib::track" => binding.resolve_track(id),
                other => anyhow::bail!("no item kind for {other}"),
            }
        })
        .collect()
}

/// The names of the covers a mix is listed with once `count` have downloaded, by the names in
/// their addresses, in order of name.
async fn mix_cover_names(apple: &OpenApple, mix: i32, count: usize) -> Result<Vec<String>> {
    let names = |collection: &crate::messages::Collection| -> Result<Vec<String>> {
        let mut names = collection
            .cover_art_map
            .keys()
            .map(|id| {
                let url = apple.binding.resolve(AppleIdKind::CoverArt, *id)?;
                let name = url
                    .split("/art/")
                    .nth(1)
                    .and_then(|rest| rest.split('/').next())
                    .context("a cover address")?;
                Ok(name.to_owned())
            })
            .collect::<Result<Vec<_>>>()?;
        names.sort();
        Ok(names)
    };
    wait_until("the covers of the mix to download", || async {
        let by_ids = apple
            .run(&FetchCollectionByIdsRequest {
                collection_type: CollectionType::Mix,
                bake_cover_arts: true,
                ids: vec![mix],
                call_id: None,
            })
            .await?;
        let collection = by_ids.result.first().context("the mix")?;
        (collection.cover_art_map.len() >= count)
            .then(|| names(collection))
            .transpose()
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recommendation_groups_are_listed_as_locked_mixes_with_lasting_ids() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), recommending()).await?;

    // Apple's groups and items in its order: the two of one title are one, without their
    // common album twice; the groups of recent plays are one under a title of their own, and a
    // group without a title of another kind is titled as untitled; the app names both; stations
    // are left out, and so is the playlist without a name, with the group it is alone in.
    assert_eq!(
        mix_summary(&apple).await?,
        [
            ("Made for You".to_owned(), 3),
            (RECENTLY_PLAYED_GROUP.to_owned(), 3),
            (UNTITLED_GROUP.to_owned(), 1),
        ]
    );
    let titles = ["Made for You", RECENTLY_PLAYED_GROUP, UNTITLED_GROUP, "Unknown"];
    let groups = mix_groups(&apple, &titles).await?;
    assert!(mix_groups(&apple, &["Gone"]).await?[0].collections.is_empty());
    assert_eq!(
        mix_names(&groups[0]),
        ["Morning Picks", "Dawn Remixes", "Dawn Album"]
    );
    // The titled and the untitled group of recent plays are one.
    assert_eq!(
        mix_names(&groups[1]),
        ["Dawn Remixes", "Dawn Album", "Morning Picks"]
    );
    assert_eq!(mix_names(&groups[2]), ["Morning Picks"]);
    assert!(groups[3].collections.is_empty());
    // Each mix is locked and plays the one album or playlist it is named after.
    for collection in groups.iter().flat_map(|group| &group.collections) {
        assert_eq!(collection.collection_type, CollectionType::Mix);
        assert!(collection.readonly);
        assert_eq!(collection.queries.len(), 1);
    }
    let picks = groups[0].collections[0].clone();
    let remixes = groups[0].collections[1].clone();
    assert_eq!(remixes.queries[0].operator, "lib::album");
    assert_eq!(picks.queries[0].operator, "lib::playlist");
    // What is recommended in two groups is one mix.
    assert_eq!(groups[1].collections[0].id, remixes.id);
    assert_eq!(groups[1].collections[2].id, picks.id);
    assert_eq!(groups[2].collections[0].id, picks.id);
    // The album opens by the id the mix's query names, from what the recommendations listed.
    let album = apple
        .run(&FetchCollectionByIdsRequest {
            collection_type: CollectionType::Album,
            bake_cover_arts: false,
            ids: vec![remixes.queries[0].parameter.parse()?],
            call_id: None,
        })
        .await?;
    assert_eq!(album.result[0].name, "Dawn Remixes");
    assert_eq!(apple.fake.count("/v1/catalog/it/albums"), 0);
    wait_until("the covers of the mixes to download", || async {
        let groups = mix_groups(&apple, &["Made for You"]).await?;
        Ok(groups[0]
            .collections
            .iter()
            .all(|mix| mix.cover_art_map.contains_key(&mix.id))
            .then_some(()))
    })
    .await?;

    // Asked again, the page lists the same mixes by the same ids.
    let ids = |groups: &[CollectionGroup]| {
        groups
            .iter()
            .map(|group| group.collections.iter().map(|mix| mix.id).collect())
            .collect::<Vec<Vec<i32>>>()
    };
    mix_summary(&apple).await?;
    assert_eq!(ids(&mix_groups(&apple, &titles).await?), ids(&groups));

    // A mix is locked, in the group it was first listed in, and plays in the player's mode.
    let mix = apple
        .run(&GetMixByIdRequest {
            mix_id: remixes.id,
            call_id: None,
        })
        .await?
        .mix;
    assert_eq!(
        (
            mix.id,
            mix.name.as_str(),
            mix.group.as_str(),
            mix.locked,
            mix.mode
        ),
        (remixes.id, "Dawn Remixes", "Made for You", true, None)
    );
    let stored = apple
        .run(&FetchMixQueriesRequest {
            mix_id: picks.id,
            call_id: None,
        })
        .await?
        .result;
    assert_eq!(stored.len(), 1);
    assert_eq!(
        (stored[0].operator.as_str(), stored[0].parameter.as_str()),
        ("lib::playlist", picks.queries[0].parameter.as_str())
    );
    assert!(stored[0].available && stored[0].target_uuid.is_none());

    // By ids, the mixes the recommendations list, in order; others are left out.
    let by_ids = apple
        .run(&FetchCollectionByIdsRequest {
            collection_type: CollectionType::Mix,
            bake_cover_arts: false,
            ids: vec![picks.id, 9999, remixes.id],
            call_id: None,
        })
        .await?;
    assert_eq!(
        by_ids
            .result
            .iter()
            .map(|mix| (mix.id, mix.name.as_str(), mix.readonly))
            .collect::<Vec<_>>(),
        [
            (picks.id, "Morning Picks", true),
            (remixes.id, "Dawn Remixes", true)
        ]
    );
    // A mix they do not list is gone, as a removed mix of Rune's own is.
    let missing = [
        apple
            .run(&GetMixByIdRequest {
                mix_id: 9999,
                call_id: None,
            })
            .await
            .err(),
        apple
            .run(&FetchMixQueriesRequest {
                mix_id: 9999,
                call_id: None,
            })
            .await
            .err(),
    ];
    for error in missing {
        assert!(not_found(&error.context("an unknown mix")?));
    }
    // Apple was asked once for all of this: the library keeps what it recommends a while.
    assert_eq!(apple.fake.count("/v1/me/recommendations"), 1);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_mixes_page_reserves_the_groups_apple_fills() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), recommending()).await?;
    let dawn = apple.binding.intern(AppleIdKind::Album, "l.alb1")?;
    create_mix(&apple, "Dawn", vec![query("lib::album", dawn)]).await?;
    apple
        .run(&crate::messages::CreateMixRequest {
            name: "Mine Too".into(),
            group: "Made for You".into(),
            mode: None,
            queries: vec![query("lib::album", dawn)],
            call_id: None,
        })
        .await?;
    let summary = apple
        .run(&FetchCollectionGroupSummaryRequest {
            collection_type: CollectionType::Mix,
            call_id: None,
        })
        .await?;
    // Rune's groups first, in the order first used; a group of Apple's title is one with
    // Apple's, which the library fills, so no mix can be put in it.
    assert_eq!(
        summary
            .groups
            .iter()
            .map(|group| (group.group_title.as_str(), group.count))
            .collect::<Vec<_>>(),
        [
            ("Favorite", 1),
            ("Made for You", 4),
            (RECENTLY_PLAYED_GROUP, 3),
            (UNTITLED_GROUP, 1),
        ]
    );
    assert_eq!(
        summary.reserved_groups,
        ["Made for You", RECENTLY_PLAYED_GROUP, UNTITLED_GROUP]
    );
    // No other page reserves a group.
    let albums = apple
        .run(&FetchCollectionGroupSummaryRequest {
            collection_type: CollectionType::Album,
            call_id: None,
        })
        .await?;
    assert!(albums.reserved_groups.is_empty());
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_recommended_mix_lists_and_plays_the_songs_of_its_album_or_playlist() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), recommending()).await?;
    mix_summary(&apple).await?;
    let groups = mix_groups(&apple, &["Made for You"]).await?;
    let (picks, remixes) = (groups[0].collections[0].id, groups[0].collections[1].id);
    let page = |queries| {
        let mut page = mix("lib::all", "true");
        page.queries = queries;
        page
    };
    // The album's songs come by disc and number.
    let album = apple
        .run(&page(stored_queries(&apple, remixes).await?))
        .await?;
    assert_eq!(
        album
            .files
            .iter()
            .map(|file| (file.track.as_str(), file.title.as_str()))
            .collect::<Vec<_>>(),
        [("catalog:1601", "Remix One"), ("catalog:1602", "Remix Two")]
    );
    let picks_queries = stored_queries(&apple, picks).await?;
    let playlist = apple.run(&page(picks_queries.clone())).await?;
    assert_eq!(
        playlist
            .files
            .iter()
            .map(|file| file.title.as_str())
            .collect::<Vec<_>>(),
        ["Dawn Chorus", "Catalog Tune"]
    );
    // The page of a mix that names a playlist pages through it as a playlist page does.
    let picks_playlist = picks_queries[0].parameter.parse()?;
    let occurrences = apple.run(&playlist_page(picks_playlist)).await?;
    assert_eq!(
        occurrences
            .entries
            .iter()
            .map(|entry| entry.track_uuid.as_str())
            .collect::<Vec<_>>(),
        ["catalog:900", "catalog:777"]
    );
    // Its playlist is one of Apple Music's, which keeps its songs.
    let opened = apple
        .run(&GetPlaylistByIdRequest {
            playlist_id: picks_playlist,
            call_id: None,
        })
        .await?
        .playlist;
    assert_eq!(
        (opened.name.as_str(), opened.group.as_str()),
        ("Morning Picks", "Apple Music")
    );
    let by_ids = apple
        .run(&FetchCollectionByIdsRequest {
            collection_type: CollectionType::Playlist,
            bake_cover_arts: false,
            ids: vec![picks_playlist],
            call_id: None,
        })
        .await?;
    assert_eq!(
        (by_ids.result[0].name.as_str(), by_ids.result[0].readonly),
        ("Morning Picks", true)
    );
    // Played from its second song, the mix plays all of its songs from there.
    let resolved = apple
        .run(&ResolveMixQueryRequest {
            queries: picks_queries,
            hint_position: 1,
            initial_playback_item: Some(song_ref(&apple, "catalog:777")),
            instantly_play: true,
            fallback_playing_items: Vec::new(),
            excluded: ExcludedItems::default(),
            call_id: None,
        })
        .await?;
    assert_eq!(
        resolved.playing_items,
        [
            song_ref(&apple, "catalog:900"),
            song_ref(&apple, "catalog:777")
        ]
    );
    assert_eq!(resolved.start_index, Some(1));
    // Each first page read the songs again; the later reads took what was kept.
    assert_eq!(
        apple.fake.count("/v1/catalog/it/playlists/pl.cat1/tracks"),
        2
    );
    // Nothing of it is a playlist Rune can change.
    let refused = add_song(&apple, picks_playlist, "i.s1", None)
        .await
        .err()
        .context("a song added to a catalog playlist")?;
    assert!(unsupported(&refused), "{refused:#}");
    assert!(apple.fake.writes.lock().unwrap().is_empty());
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_recommended_mix_cannot_be_changed_or_removed() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), recommending()).await?;
    mix_summary(&apple).await?;
    let mix = mix_groups(&apple, &["Made for You"]).await?[0].collections[0].id;
    let update = |mix_id| UpdateMixRequest {
        mix_id,
        name: "Renamed".into(),
        group: String::new(),
        mode: None,
        queries: vec![query("lib::all", "true")],
        call_id: None,
    };
    let refused = [
        apple.run(&update(mix)).await.err(),
        apple
            .run(&RemoveMixRequest {
                mix_id: mix,
                call_id: None,
            })
            .await
            .err(),
        apple
            .run(&AddItemToMixRequest {
                mix_id: mix,
                operator: "lib::track".into(),
                parameter: apple.binding.intern_track("i.s1")?.to_string(),
                call_id: None,
            })
            .await
            .err(),
    ];
    for error in refused {
        assert!(unsupported(&error.context("a refused mix request")?));
    }
    // An id that names no mix is an input error.
    let unknown = apple.run(&update(9999)).await.err().context("no mix")?;
    assert!(invalid_input(&unknown), "{unknown:#}");
    assert!(apple.fake.writes.lock().unwrap().is_empty());
    // The mix is as it was, and is not one the menus offer to add to.
    let kept = apple
        .run(&GetMixByIdRequest {
            mix_id: mix,
            call_id: None,
        })
        .await?
        .mix;
    assert_eq!(kept.name, "Morning Picks");
    assert!(kept.locked);
    assert!(rune_mixes(&apple).await?.is_empty());
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rune_mixes_show_the_covers_of_what_they_play_from_what_is_kept() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    // The Artists page was opened once, so the artist list is kept.
    let one = artist_named(&all_artists(&apple).await?, "Artist One")?;
    let night = make_rune_playlist(&apple, "Night Drive", "Mood").await?;
    for track in ["i.s1", "i.s2", "i.s3"] {
        add_song(&apple, night.id, track, None).await?;
    }
    let night_album = apple.binding.intern(AppleIdKind::Album, "l.alb2")?;
    let first = apple.binding.intern_track("i.s1")?;

    // An artist without a picture shows the covers of its songs, each album's once.
    let by_artist = create_mix(&apple, "One", vec![query("lib::artist", one)]).await?;
    // A Rune playlist shows each of its songs' covers.
    let by_playlist = create_mix(&apple, "Night", vec![query("lib::playlist", night.id)]).await?;
    // A mix of several items shows the first cover of each, the same cover once.
    let several = create_mix(
        &apple,
        "Several",
        vec![
            query("lib::album", night_album),
            query("lib::artist", one),
            query("lib::track", first),
        ],
    )
    .await?;
    assert_eq!(
        mix_cover_names(&apple, by_artist.id, 1).await?,
        ["Dawn_Album"]
    );
    assert_eq!(
        mix_cover_names(&apple, by_playlist.id, 2).await?,
        ["Dawn_Album", "Night_Album"]
    );
    assert_eq!(
        mix_cover_names(&apple, several.id, 2).await?,
        ["Dawn_Album", "Night_Album"]
    );
    // Listing them asks Apple for nothing but covers.
    let reads = apple.fake.total();
    for mix in [&by_artist, &by_playlist, &several] {
        mix_cover_names(&apple, mix.id, 1).await?;
    }
    assert_eq!(apple.fake.total(), reads);

    // The home shows the same covers, Rune's mixes in the order they were made.
    wait_until("the covers of the home's mixes", || async {
        let home = apple
            .run(&home_mixes(LibraryHomeListMode::Forward))
            .await?;
        let covers: Vec<(String, usize, bool)> = home.result[0]
            .entries
            .iter()
            .map(|entry| (entry.name.clone(), entry.cover_art_map.len(), entry.readonly))
            .collect();
        let wanted = [
            ("One".to_owned(), 1, false),
            ("Night".to_owned(), 2, false),
            ("Several".to_owned(), 2, false),
        ];
        Ok((covers == wanted).then_some(()))
    })
    .await?;
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_mix_of_an_artist_with_a_picture_shows_the_picture() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            credited: true,
            ..AppleOptions::default()
        },
    )
    .await?;
    let one = artist_named(&all_artists(&apple).await?, "Artist One")?;
    let mix = create_mix(&apple, "One", vec![query("lib::artist", one)]).await?;
    assert_eq!(
        mix_cover_names(&apple, mix.id, 1).await?,
        ["Artist_One_Photo"]
    );
    apple.finish().await
}

/// What the restarted node lists as Rune's mixes, by name, with the keys of what each plays and
/// the titles of its songs, as its binding numbers them.
async fn restarted_mixes(
    ui: &crate::utils::GlobalParams,
    binding: &AppleMusicBinding,
) -> Result<Vec<(String, Vec<String>, Vec<String>)>> {
    let mixes = dispatch_gui_request(ui, &FetchAllMixesRequest { call_id: None })
        .await?
        .context("the mixes")?
        .mixes;
    let mut listed = Vec::new();
    for found in mixes {
        let queries: Vec<MixQuery> = dispatch_gui_request(
            ui,
            &FetchMixQueriesRequest {
                mix_id: found.id,
                call_id: None,
            },
        )
        .await?
        .context("the queries")?
        .result
        .into_iter()
        .map(|stored| query(&stored.operator, stored.parameter))
        .collect();
        let mut page = mix("lib::all", "true");
        page.queries = queries.clone();
        let titles = dispatch_gui_request(ui, &page)
            .await?
            .context("the songs")?
            .files
            .into_iter()
            .map(|file| file.title)
            .collect();
        listed.push((found.name, query_keys(binding, &queries)?, titles));
    }
    Ok(listed)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_rune_mix_lasts_through_a_new_binding_by_apples_keys() -> Result<()> {
    use crate::service::device_records::device_records_folder;
    use crate::service::rune_mixes::MIXES_FILE;

    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let file = device_records_folder(Path::new(&*apple.node.config_path), apple.record.0)
        .join(MIXES_FILE);
    // The track menu's New Mix makes the mix, then adds the track to it.
    let made = create_mix(&apple, "Night", Vec::new()).await?;
    for track in ["i.s1", "catalog:900"] {
        apple
            .run(&AddItemToMixRequest {
                mix_id: made.id,
                operator: "lib::track".into(),
                parameter: apple.binding.intern_track(track)?.to_string(),
                call_id: None,
            })
            .await?;
    }
    let queries = stored_queries(&apple, made.id).await?;
    assert_eq!(
        query_keys(&apple.binding, &queries)?,
        ["i.s1", "catalog:900"]
    );
    assert_eq!(
        mix_titles(&apple, queries.clone()).await?,
        ["First Light", "Dawn Chorus"]
    );
    // The file holds Apple's keys, not the ids this binding gave.
    let written = std::fs::read_to_string(&file)?;
    assert!(written.contains("\"i.s1\"") && written.contains("\"catalog:900\""));
    for query in &queries {
        assert!(!written.contains(&format!("\"{}\"", query.parameter)));
    }
    assert!(apple.fake.writes.lock().unwrap().is_empty());

    // A binding made afresh, as after a restart, numbers what it lists anew.
    let restarted = restart_apple(apple).await?;
    let (ui, binding) = restarted.open().await?;
    for n in 0..5 {
        binding.intern_track(&format!("i.other{n}"))?;
    }
    restarted.sign_in(USER_TOKEN).await?;
    assert_eq!(
        restarted_mixes(&ui, &binding).await?,
        [(
            "Night".to_owned(),
            vec!["i.s1".to_owned(), "catalog:900".to_owned()],
            vec!["First Light".to_owned(), "Dawn Chorus".to_owned()]
        )]
    );
    restarted.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_item_added_to_a_mix_changed_meanwhile_is_refused() -> Result<()> {
    use crate::apple_music_artists::named_artist_remote;
    use crate::messages::UpdateMixRequest;

    const ARTISTS: &str = "/v1/me/library/artists";
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let s1 = apple.binding.intern_track("i.s1")?;
    let made = create_mix(&apple, "Night", vec![query("lib::track", s1)]).await?;
    // Adding an artist reads the artist list, whose answer Apple holds.
    let artist = apple.binding.intern(
        AppleIdKind::Artist,
        &named_artist_remote("Artist One").context("an artist key")?,
    )?;
    let release = Arc::new(tokio::sync::Notify::new());
    *apple.fake.hold_once.lock().unwrap() = Some((ARTISTS.into(), release.clone()));
    let reads = apple.fake.count(ARTISTS);
    let adding = AddItemToMixRequest {
        mix_id: made.id,
        operator: "lib::artist".into(),
        parameter: artist.to_string(),
        call_id: None,
    };
    let add = apple.run(&adding);
    let change = async {
        fake_asked(&apple, ARTISTS, reads + 1).await?;
        // Meanwhile the mix is given other queries.
        let s2 = apple.binding.intern_track("i.s2")?;
        apple
            .run(&UpdateMixRequest {
                mix_id: made.id,
                name: "Night".into(),
                group: String::new(),
                mode: None,
                queries: vec![query("lib::track", s2)],
                call_id: None,
            })
            .await?;
        release.notify_one();
        Ok::<_, anyhow::Error>(())
    };
    let (added, changed) = tokio::join!(add, change);
    changed?;
    assert!(added.is_err());
    let queries = stored_queries(&apple, made.id).await?;
    assert_eq!(query_keys(&apple.binding, &queries)?, ["i.s2"]);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn catalogue_items_in_rune_mixes_and_playlists_last_through_a_new_binding() -> Result<()> {
    use crate::service::device_records::device_records_folder;
    use crate::service::rune_mixes::MIXES_FILE;

    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let folder = device_records_folder(Path::new(&*apple.node.config_path), apple.record.0);
    // A mix of a catalog album and playlist, and a playlist of a catalog song.
    let album = apple.binding.intern(AppleIdKind::Album, "catalog:1600")?;
    let playlist = apple
        .binding
        .intern(AppleIdKind::Playlist, "catalog:pl.cat1")?;
    let made = create_mix(
        &apple,
        "Catalog",
        vec![query("lib::album", album), query("lib::playlist", playlist)],
    )
    .await?;
    let mine = make_rune_playlist(&apple, "Mine", "Favorite").await?;
    add_song(&apple, mine.id, "catalog:777", None).await?;
    let titles = ["Remix One", "Remix Two", "Dawn Chorus", "Catalog Tune"];
    assert_eq!(
        mix_titles(&apple, stored_queries(&apple, made.id).await?).await?,
        titles
    );
    // Nothing was written to the account, and the files hold Apple's keys, not session ids.
    assert!(apple.fake.writes.lock().unwrap().is_empty());
    let written = std::fs::read_to_string(folder.join(MIXES_FILE))?;
    for key in ["catalog:1600", "catalog:pl.cat1"] {
        assert!(written.contains(&format!("\"{key}\"")), "{written}");
    }
    let kept = std::fs::read_to_string(playlists_file(&apple))?;
    assert!(kept.contains("\"catalog:777\""), "{kept}");

    // A binding made afresh, as after a restart, numbers what it lists anew.
    let restarted = restart_apple(apple).await?;
    let (ui, binding) = restarted.open().await?;
    for n in 0..5 {
        binding.intern(AppleIdKind::Album, &format!("l.other{n}"))?;
    }
    restarted.sign_in(USER_TOKEN).await?;
    let listed = restarted_mixes(&ui, &binding).await?;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].2, titles);
    let playlists = dispatch_gui_request(&ui, &FetchAllPlaylistsRequest { call_id: None })
        .await?
        .context("the playlists")?
        .playlists;
    let page = dispatch_gui_request(&ui, &playlist_page(playlists[0].id))
        .await?
        .context("the playlist page")?;
    assert_eq!(page.entries[0].track_uuid, "catalog:777");
    restarted.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_rune_mix_is_renamed_refilled_and_removed_and_refuses_queries_it_cannot_play()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let (apple, heard) = open_apple_heard(dir.path()).await?;
    let dawn = apple.binding.intern(AppleIdKind::Album, "l.alb1")?;
    let night = apple.binding.intern(AppleIdKind::Album, "l.alb2")?;
    // Every change tells the UI once; a refused one does not. Nothing here lists songs, whose
    // covers would tell it too.
    let mut told = told_settled(&heard).await;
    let made = create_mix(&apple, "Dawn", vec![query("lib::album", dawn)]).await?;
    told = told_more(&heard, told, 1, "create").await?;
    assert_eq!(
        query_keys(&apple.binding, &stored_queries(&apple, made.id).await?)?,
        ["l.alb1"]
    );

    // A genre is a query the translation cannot play, so it is refused and the mix is as it
    // was.
    let refused = apple
        .run(&AddItemToMixRequest {
            mix_id: made.id,
            operator: "lib::genre".into(),
            parameter: "1".into(),
            call_id: None,
        })
        .await
        .err()
        .context("a genre")?;
    assert!(unsupported(&refused));
    assert_eq!(stored_queries(&apple, made.id).await?.len(), 1);
    told = told_more(&heard, told, 0, "a refused query").await?;

    let updated = apple
        .run(&UpdateMixRequest {
            mix_id: made.id,
            name: "Night".into(),
            group: "Mood".into(),
            mode: Some(crate::messages::PlaybackMode::Shuffle),
            queries: vec![query("lib::album", night)],
            call_id: None,
        })
        .await?
        .mix;
    told_more(&heard, told, 1, "update").await?;
    assert_eq!((updated.id, updated.name.as_str()), (made.id, "Night"));
    let kept = apple
        .run(&GetMixByIdRequest {
            mix_id: made.id,
            call_id: None,
        })
        .await?
        .mix;
    assert_eq!(kept.group, "Mood");
    assert_eq!(kept.mode, Some(crate::messages::PlaybackMode::Shuffle));
    assert!(!kept.locked);
    assert_eq!(
        mix_titles(&apple, stored_queries(&apple, made.id).await?).await?,
        ["Third Road"]
    );

    apple
        .run(&RemoveMixRequest {
            mix_id: made.id,
            call_id: None,
        })
        .await?;
    assert!(rune_mixes(&apple).await?.is_empty());
    let gone = apple
        .run(&GetMixByIdRequest {
            mix_id: made.id,
            call_id: None,
        })
        .await
        .err()
        .context("a removed mix")?;
    assert!(not_found(&gone), "{gone:#}");
    assert!(apple.fake.writes.lock().unwrap().is_empty());
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_mix_of_two_artists_lists_and_plays_both_and_pages_through_them() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let one = apple
        .binding
        .intern(AppleIdKind::Artist, "name:artist one")?;
    let two = apple
        .binding
        .intern(AppleIdKind::Artist, "name:artist two")?;
    let made = create_mix(
        &apple,
        "Both",
        vec![query("lib::artist", one), query("lib::artist", two)],
    )
    .await?;
    let queries = stored_queries(&apple, made.id).await?;
    // The artist Apple names a catalog artist for is kept by it.
    assert_eq!(
        query_keys(&apple.binding, &queries)?,
        ["artist:300", "name:artist two"]
    );
    assert_eq!(
        mix_titles(&apple, queries.clone()).await?,
        ["First Light", "Second Wind", "Third Road"]
    );
    // A page at a time, as the mix page scrolls.
    let mut page = mix("lib::all", "true");
    page.queries = queries.clone();
    page.page_size = 2;
    let titles = |files: Vec<crate::messages::MediaFile>| {
        files.into_iter().map(|file| file.title).collect::<Vec<_>>()
    };
    assert_eq!(
        titles(apple.run(&page).await?.files),
        ["First Light", "Second Wind"]
    );
    page.cursor = 2;
    assert_eq!(titles(apple.run(&page).await?.files), ["Third Road"]);
    page.cursor = 3;
    assert!(apple.run(&page).await?.files.is_empty());
    let resolved = apple
        .run(&ResolveMixQueryRequest {
            queries,
            hint_position: 2,
            initial_playback_item: Some(song_ref(&apple, "i.s3")),
            instantly_play: true,
            fallback_playing_items: Vec::new(),
            excluded: ExcludedItems::default(),
            call_id: None,
        })
        .await?;
    assert_eq!(
        resolved.playing_items,
        [
            song_ref(&apple, "i.s1"),
            song_ref(&apple, "i.s2"),
            song_ref(&apple, "i.s3")
        ]
    );
    assert_eq!(resolved.start_index, Some(2));
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_album_and_tracks_together_list_the_tracks_first_each_song_once() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            bare_night: true,
            ..AppleOptions::default()
        },
    )
    .await?;
    let night = apple.binding.intern(AppleIdKind::Album, "l.alb2")?;
    let track = |key: &str| -> Result<MixQuery> {
        Ok(query("lib::track", apple.binding.intern_track(key)?))
    };
    let queries = vec![
        query("lib::album", night),
        track("i.s3")?,
        track("i.s1")?,
    ];
    // "Third Road" is the album's song as well, and comes once, as a track.
    assert_eq!(
        mix_titles(&apple, queries.clone()).await?,
        ["Third Road", "First Light"]
    );
    // The cover filter goes with them: the Night Album has no cover.
    let mut covered = queries;
    covered.push(query("filter::with_cover_art", "true"));
    assert_eq!(mix_titles(&apple, covered).await?, ["First Light"]);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_item_added_to_a_rune_mix_of_a_collection_joins_it() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let dawn = apple.binding.intern(AppleIdKind::Album, "l.alb1")?;
    let made = create_mix(&apple, "Dawn", vec![query("lib::album", dawn)]).await?;
    // The collection menu's Add to Mix names the artist, then the track menu's a song.
    for (operator, parameter) in [
        (
            "lib::artist",
            apple
                .binding
                .intern(AppleIdKind::Artist, "name:artist two")?,
        ),
        ("lib::track", apple.binding.intern_track("catalog:900")?),
    ] {
        apple
            .run(&AddItemToMixRequest {
                mix_id: made.id,
                operator: operator.into(),
                parameter: parameter.to_string(),
                call_id: None,
            })
            .await?;
    }
    let queries = stored_queries(&apple, made.id).await?;
    assert_eq!(
        query_keys(&apple.binding, &queries)?,
        ["l.alb1", "name:artist two", "catalog:900"]
    );
    assert_eq!(
        mix_titles(&apple, queries).await?,
        ["Dawn Chorus", "First Light", "Second Wind", "Third Road"]
    );
    assert!(apple.fake.writes.lock().unwrap().is_empty());
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_home_lists_the_recommended_mixes_by_the_ids_of_the_mixes_page() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), recommending()).await?;
    let forward = apple.run(&home_mixes(LibraryHomeListMode::Forward)).await?;
    // Apple's order, each mix once.
    assert_eq!(
        entry_names(&forward),
        ["Morning Picks", "Dawn Remixes", "Dawn Album"]
    );
    let entries = &forward.result[0].entries;
    for entry in entries {
        assert_eq!(entry.collection_type, CollectionType::Mix);
        assert!(entry.readonly);
    }
    mix_summary(&apple).await?;
    let page = mix_groups(&apple, &["Made for You"]).await?;
    let opened = |id: i32, queries: &[MixQuery]| (id, queries[0].parameter.clone());
    assert_eq!(
        entries
            .iter()
            .map(|entry| opened(entry.id, &entry.queries))
            .collect::<Vec<_>>(),
        page[0]
            .collections
            .iter()
            .map(|mix| opened(mix.id, &mix.queries))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        entry_names(&apple.run(&home_mixes(LibraryHomeListMode::Reverse)).await?),
        ["Dawn Album", "Dawn Remixes", "Morning Picks"]
    );
    assert_eq!(
        entry_names(&apple.run(&home_mixes(LibraryHomeListMode::Name)).await?),
        ["Dawn Album", "Dawn Remixes", "Morning Picks"]
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_recommendations_leave_the_home_mixes_empty_unless_the_sign_in_is_lost() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            empty: true,
            recommends: true,
            ..AppleOptions::default()
        },
    )
    .await?;
    *apple.fake.respond_path.lock().unwrap() = Some((
        "/v1/me/recommendations".into(),
        400,
        r#"{"errors":[{"status":"400","code":"PARAMETER_ERROR"}]}"#.into(),
    ));
    let mode = LibraryHomeListMode::Forward;
    let mut home = home_mixes(mode);
    home.queries.insert(
        0,
        ComplexQuery {
            title: "Albums".into(),
            query: LibraryHomeQuery::Albums(mode),
        },
    );
    // Nothing else has an entry, and still the home answers.
    let response = apple.run(&home).await?;
    assert_eq!(response.result.len(), 2);
    assert!(response.result.iter().all(|group| group.entries.is_empty()));
    // The Mixes page tells of the failure.
    assert!(mix_summary(&apple).await.is_err());

    *apple.fake.respond_path.lock().unwrap() = None;
    assert_eq!(apple.run(&home).await?.result[1].entries.len(), 3);

    // A sign-in that must be renewed fails the home, as for its other groups. What was
    // recommended went with the sign-out, so the new sign-in reads it again.
    apple.fake.reject_everything.store(true, Ordering::SeqCst);
    apple.binding.sign_out();
    apple
        .binding
        .sign_in(Tokens::new(DEVELOPER_TOKEN, APPLE_OTHER_TOKEN).context("tokens")?);
    let error = apple
        .run(&home_mixes(mode))
        .await
        .err()
        .context("a lost sign-in")?;
    assert_eq!(
        request_failure_cause(&error),
        Some(RequestFailureCause::ServiceSignInRequired {
            record_id: apple.record.0.to_string(),
        })
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recommendations_read_before_a_sign_out_are_not_kept_after_it() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), recommending()).await?;
    mix_summary(&apple).await?;
    let mix = mix_groups(&apple, &["Made for You"]).await?[0].collections[0].id;
    assert!(apple.binding.recommended_mix(mix).is_some());
    let asked = || apple.fake.count("/v1/me/recommendations");

    // A sign-out forgets the mixes, and their ids stay; the next sign-in reads them again.
    apple.binding.sign_out();
    assert!(apple.binding.recommended_mix(mix).is_none());
    apple
        .binding
        .sign_in(Tokens::new(DEVELOPER_TOKEN, APPLE_OTHER_TOKEN).context("tokens")?);
    let reads = asked();
    let found = apple
        .run(&GetMixByIdRequest {
            mix_id: mix,
            call_id: None,
        })
        .await?
        .mix;
    assert_eq!(found.name, "Morning Picks");
    assert_eq!(asked(), reads + 1);

    // A read begun before a sign-out is not kept after it. Apple holds its answer until the
    // sign-out is done.
    apple.binding.sign_out();
    apple
        .binding
        .sign_in(Tokens::new(DEVELOPER_TOKEN, USER_TOKEN).context("tokens")?);
    let release = Arc::new(tokio::sync::Notify::new());
    *apple.fake.hold_once.lock().unwrap() =
        Some(("/v1/me/recommendations".into(), release.clone()));
    let reads = asked();
    let read = mix_summary(&apple);
    let switch = async {
        wait_until("the recommendations to be asked for", || async {
            Ok((asked() > reads).then_some(()))
        })
        .await?;
        apple.binding.sign_out();
        apple
            .binding
            .sign_in(Tokens::new(DEVELOPER_TOKEN, APPLE_OTHER_TOKEN).context("tokens")?);
        release.notify_one();
        Ok::<_, anyhow::Error>(())
    };
    let (read, switch) = tokio::join!(read, switch);
    switch?;
    // The caller that waited still gets what was read, and the library keeps none of it, so the
    // next page reads it again.
    assert_eq!(read?.len(), 3);
    assert!(apple.binding.recommended_mix(mix).is_none());
    assert_eq!(asked(), reads + 1);
    mix_summary(&apple).await?;
    assert_eq!(asked(), reads + 2);
    apple.finish().await
}

// Likes kept in Rune, the one-time import, and records that outlive the record -----------------

fn like(apple: &OpenApple, track: &str, liked: bool) -> SetLikedRequest {
    SetLikedRequest {
        item: Some(song_ref(apple, track)),
        liked,
        call_id: None,
    }
}

async fn is_liked(apple: &OpenApple, track: &str) -> Result<bool> {
    Ok(apple
        .run(&crate::messages::GetLikedRequest {
            item: Some(song_ref(apple, track)),
            call_id: None,
        })
        .await?
        .liked)
}

/// The names of the home's Liked group, in order.
async fn liked_group(apple: &OpenApple) -> Result<Vec<String>> {
    let home = apple
        .run(&ComplexQueryRequest {
            queries: vec![ComplexQuery {
                title: "Liked".into(),
                query: LibraryHomeQuery::Liked,
            }],
            call_id: None,
        })
        .await?;
    Ok(home.result[0]
        .entries
        .iter()
        .map(|entry| entry.name.clone())
        .collect())
}

fn liking(likes: Vec<&'static str>) -> AppleOptions {
    AppleOptions {
        apple_likes: likes,
        ..AppleOptions::default()
    }
}

fn with_playlists() -> AppleOptions {
    AppleOptions {
        playlists: true,
        ..AppleOptions::default()
    }
}

/// Waits for the import the last sign-in of `binding` started to end: once everything was
/// imported, it asks Apple nothing, which the counts of what Apple was asked then show.
async fn import_settled(binding: &AppleMusicBinding) -> Result<()> {
    wait_until("the import of the sign-in to end", || async {
        Ok((!binding.importing.load(Ordering::SeqCst)).then_some(()))
    })
    .await
}

/// Makes a binding of the library afresh, as at the next start: the one held is retired, the
/// library is opened again and signed in. Answers the UI's params, which the test holds, and
/// the binding.
async fn bind_again(
    apple: &OpenApple,
) -> Result<(Arc<crate::utils::GlobalParams>, Arc<AppleMusicBinding>)> {
    apple.node.services.retire(apple.record).await;
    let ui =
        super::open_registered_library_record(apple.node.clone(), apple.record.0.to_string())
            .await?;
    let binding = apple
        .node
        .services
        .apple_music_binding(apple.record)
        .context("the new binding")?;
    sign_in_again(apple, USER_TOKEN).await?;
    Ok((ui, binding))
}

/// Signs the open Apple library in again as the platform does, which starts an import.
async fn sign_in_again(apple: &OpenApple, user_token: &str) -> Result<()> {
    use crate::messages::{ServiceSignIn, SignInServiceLibraryRequest};
    use crate::utils::sign_in_service_library_on_node;

    sign_in_service_library_on_node(
        &SignInServiceLibraryRequest {
            record_id: apple.record.0.to_string(),
            sign_in: ServiceSignIn::Platform {
                user_token: user_token.into(),
                developer_token: Some(DEVELOPER_TOKEN.into()),
            },
            call_id: None,
        },
        &apple.node,
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn likes_are_kept_in_rune_and_never_sent_to_apple() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let reads = apple.fake.rating_reads.load(Ordering::SeqCst);
    assert!(!is_liked(&apple, "i.s1").await?);

    let liked = apple.run(&like(&apple, "i.s1", true)).await?;
    assert!(liked.liked);
    assert_eq!(liked.item, song_ref(&apple, "i.s1"));
    apple.run(&like(&apple, "catalog:777", true)).await?;
    assert!(is_liked(&apple, "i.s1").await?);
    assert!(is_liked(&apple, "catalog:777").await?);
    assert!(!is_liked(&apple, "i.s2").await?);

    apple.run(&like(&apple, "i.s1", false)).await?;
    apple.run(&like(&apple, "catalog:777", false)).await?;
    assert!(!is_liked(&apple, "i.s1").await?);
    assert!(!is_liked(&apple, "catalog:777").await?);
    apple.run(&like(&apple, "i.s1", true)).await?;
    assert!(is_liked(&apple, "i.s1").await?);
    // Apple was told nothing and asked nothing about ratings.
    assert!(apple.fake.writes.lock().unwrap().is_empty());
    assert_eq!(apple.fake.rating_reads.load(Ordering::SeqCst), reads);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_apple_item_is_liked_while_a_local_library_is_open() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    open_own_library(&apple).await?;
    assert!(apple.node.service_binding(apple.record.0)?.is_none());

    let liked = apple.run(&like(&apple, "i.s2", true)).await?;
    assert!(liked.liked);
    assert!(apple.fake.writes.lock().unwrap().is_empty());
    assert!(is_liked(&apple, "i.s2").await?);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_like_belongs_to_the_song_not_to_its_library_or_catalog_copy() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    // "i.s1" and "catalog:ci.s1" are one song: the library's copy names its catalog song.
    assert!(!is_liked(&apple, "i.s1").await?);
    apple.run(&like(&apple, "catalog:ci.s1", true)).await?;
    assert!(is_liked(&apple, "i.s1").await?);
    assert!(is_liked(&apple, "catalog:ci.s1").await?);
    assert_eq!(liked_group(&apple).await?, ["First Light"]);
    // Unliking the library copy unlikes the catalog song.
    apple.run(&like(&apple, "i.s1", false)).await?;
    assert!(!is_liked(&apple, "i.s1").await?);
    assert!(!is_liked(&apple, "catalog:ci.s1").await?);
    assert!(liked_group(&apple).await?.is_empty());
    // And the other way round.
    apple.run(&like(&apple, "i.s1", true)).await?;
    assert!(is_liked(&apple, "catalog:ci.s1").await?);
    apple.run(&like(&apple, "catalog:ci.s1", false)).await?;
    assert!(!is_liked(&apple, "i.s1").await?);
    // A song with one known key is as before.
    apple.run(&like(&apple, "catalog:777", true)).await?;
    assert!(is_liked(&apple, "catalog:777").await?);
    assert!(!is_liked(&apple, "i.s2").await?);
    assert!(apple.fake.writes.lock().unwrap().is_empty());
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn likes_stored_under_either_key_count_and_the_home_shows_the_library_copy_once()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    // Entries made under each key of a song, as before the other key was known.
    apple.binding.rune_likes.set_liked(&["i.s1"], true).await?;
    assert!(is_liked(&apple, "catalog:ci.s1").await?);
    apple
        .binding
        .rune_likes
        .set_liked(&["catalog:ci.s1"], true)
        .await?;
    apple.binding.rune_likes.set_liked(&["i.s2"], true).await?;
    assert_eq!(liked_group(&apple).await?, ["Second Wind", "First Light"]);
    // An unlike counts for both, whichever entry is older.
    apple.run(&like(&apple, "i.s1", false)).await?;
    assert!(!is_liked(&apple, "catalog:ci.s1").await?);
    assert_eq!(liked_group(&apple).await?, ["Second Wind"]);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_import_does_not_like_again_a_song_unliked_under_its_other_key() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), liking(vec!["i.s1", "i.s2", "i.s3"])).await?;
    let folder = records_of(&apple, apple.record);
    // The import liked the songs by one key each, and either key finds them.
    assert!(is_liked(&apple, "catalog:ci.s3").await?);
    assert_eq!(
        liked_group(&apple).await?,
        ["Third Road", "Second Wind", "First Light"]
    );
    // A library that was not imported yet, with an unlike noted under each song's other key.
    apple.node.services.retire(apple.record).await;
    std::fs::remove_file(folder.join("likes.imported"))?;
    std::fs::write(
        folder.join("likes.json"),
        serde_json::to_vec(&json!({
            "format": 1,
            "songs": [
                {"id": "catalog:ci.s1", "liked": false},
                {"id": "i.s2", "liked": false},
            ],
        }))?,
    )?;
    let (_ui, _binding) = bind_again(&apple).await?;
    wait_until("the likes to be imported", || async {
        Ok(folder.join("likes.imported").exists().then_some(()))
    })
    .await?;
    assert!(!is_liked(&apple, "i.s1").await?);
    assert!(!is_liked(&apple, "catalog:ci.s2").await?);
    assert!(is_liked(&apple, "i.s3").await?);
    assert_eq!(liked_group(&apple).await?, ["Third Road"]);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_home_lists_the_likes_kept_in_rune_newest_first_without_asking_for_ratings()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    // Apple has rated two songs liked, which the sign-in imported.
    let apple = open_apple_with(dir.path(), liking(vec!["i.s1", "i.s2"])).await?;
    let reads = apple.fake.rating_reads.load(Ordering::SeqCst);
    assert_eq!(liked_group(&apple).await?, ["Second Wind", "First Light"]);

    // A song liked in Rune is the newest; one unliked is gone; a song the binding cannot name
    // is left out, not failed on.
    apple.run(&like(&apple, "i.s3", true)).await?;
    apple.run(&like(&apple, "i.s9", true)).await?;
    apple.run(&like(&apple, "i.s2", false)).await?;
    assert_eq!(liked_group(&apple).await?, ["Third Road", "First Light"]);
    // Liked again, a song is the newest.
    apple.run(&like(&apple, "i.s2", true)).await?;
    assert_eq!(
        liked_group(&apple).await?,
        ["Second Wind", "Third Road", "First Light"]
    );
    assert_eq!(apple.fake.rating_reads.load(Ordering::SeqCst), reads);
    assert!(apple.fake.writes.lock().unwrap().is_empty());
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_likes_are_imported_from_the_songs_apple_lists_now_not_a_list_kept_before()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    // The library's song list was read and kept while the library was empty.
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            empty: true,
            apple_likes: vec!["i.s1"],
            ..AppleOptions::default()
        },
    )
    .await?;
    let folder = records_of(&apple, apple.record);
    // Its likes are not imported yet, and Apple lists its songs now. A sign-in imports them.
    std::fs::remove_file(folder.join("likes.imported"))?;
    apple.fake.empty.store(false, Ordering::SeqCst);
    sign_in_again(&apple, USER_TOKEN).await?;
    wait_until("the likes to be imported", || async {
        Ok(folder.join("likes.imported").exists().then_some(()))
    })
    .await?;
    assert!(is_liked(&apple, "i.s1").await?);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_first_sign_in_imports_apples_likes_once() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), liking(vec!["i.s1", "i.s3"])).await?;
    let folder = records_of(&apple, apple.record);
    assert!(folder.join("likes.imported").exists());
    assert!(folder.join("likes.json").exists());
    assert!(is_liked(&apple, "i.s1").await?);
    assert!(!is_liked(&apple, "i.s2").await?);
    assert!(is_liked(&apple, "i.s3").await?);
    assert_eq!(liked_group(&apple).await?, ["Third Road", "First Light"]);
    // Apple was only read from.
    assert!(apple.fake.writes.lock().unwrap().is_empty());

    // Another sign-in, and a binding made afresh as after a restart, do not read the ratings
    // again, though Apple's have changed.
    let reads = apple.fake.rating_reads.load(Ordering::SeqCst);
    assert!(reads > 0);
    apple.fake.liked.lock().unwrap().insert("i.s2".into());
    sign_in_again(&apple, USER_TOKEN).await?;
    import_settled(&apple.binding).await?;
    let (_ui, binding) = bind_again(&apple).await?;
    import_settled(&binding).await?;
    assert_eq!(apple.fake.rating_reads.load(Ordering::SeqCst), reads);
    assert!(!is_liked(&apple, "i.s2").await?);
    assert_eq!(liked_group(&apple).await?, ["Third Road", "First Light"]);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_like_or_unlike_made_in_rune_while_the_likes_are_imported_is_kept() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let release = Arc::new(tokio::sync::Notify::new());
    // Apple answers the ratings only when the test lets it, so the import is under way.
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            apple_likes: vec!["i.s1", "i.s2"],
            hold_ratings: Some(release.clone()),
            import_unfinished: true,
            ..AppleOptions::default()
        },
    )
    .await?;
    fake_asked(&apple, "/v1/me/ratings/library-songs?", 1).await?;
    // The user unlikes a song Apple has liked, and likes one Apple has not.
    apple.run(&like(&apple, "i.s2", false)).await?;
    apple.run(&like(&apple, "i.s3", true)).await?;
    release.notify_one();
    import_ended(&apple, apple.record).await?;
    assert!(is_liked(&apple, "i.s1").await?);
    assert!(!is_liked(&apple, "i.s2").await?);
    assert!(is_liked(&apple, "i.s3").await?);
    // What Rune liked is newer than what came in.
    assert_eq!(liked_group(&apple).await?, ["Third Road", "First Light"]);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sign_in_during_an_import_is_imported_for_once_the_import_ends() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let release = Arc::new(tokio::sync::Notify::new());
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            apple_likes: vec!["i.s1"],
            hold_ratings: Some(release.clone()),
            import_unfinished: true,
            ..AppleOptions::default()
        },
    )
    .await?;
    fake_asked(&apple, "/v1/me/ratings/library-songs?", 1).await?;
    let marker = records_of(&apple, apple.record).join("likes.imported");
    assert!(!marker.exists());
    // Another sign-in comes while the import waits for Apple: it stops that one, and its own
    // runs when that has ended.
    sign_in_again(&apple, APPLE_OTHER_TOKEN).await?;
    release.notify_one();
    import_ended(&apple, apple.record).await?;
    // The first import read the ratings, and the one of the new sign-in read them again.
    assert_eq!(apple.fake.rating_reads.load(Ordering::SeqCst), 2);
    assert!(is_liked(&apple, "i.s1").await?);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn likes_that_could_not_be_read_are_imported_at_the_next_sign_in() -> Result<()> {
    let dir = tempfile::tempdir()?;
    // Apple fails the ratings at the import of the first sign-in.
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            apple_likes: vec!["i.s2"],
            fail_rating_reads_from: 1,
            import_unfinished: true,
            ..AppleOptions::default()
        },
    )
    .await?;
    let folder = records_of(&apple, apple.record);
    // The import ends with the ids of the account, which come after the likes.
    wait_until("the first import to end", || async {
        Ok(folder.join("account.json").exists().then_some(()))
    })
    .await?;
    let marker = folder.join("likes.imported");
    assert!(folder.join("playlists.imported").exists());
    assert!(!marker.exists());
    assert!(!is_liked(&apple, "i.s2").await?);
    // The user likes a song meanwhile.
    apple.run(&like(&apple, "i.s1", true)).await?;

    apple.fake.fail_rating_reads_from.store(0, Ordering::SeqCst);
    sign_in_again(&apple, USER_TOKEN).await?;
    wait_until("the likes to be imported", || async {
        Ok(marker.exists().then_some(()))
    })
    .await?;
    assert!(is_liked(&apple, "i.s2").await?);
    assert!(is_liked(&apple, "i.s1").await?);
    assert_eq!(liked_group(&apple).await?, ["First Light", "Second Wind"]);
    apple.finish().await
}

/// Rune's playlists by name, with the tracks of their entries in order.
async fn rune_playlist_songs(apple: &OpenApple) -> Result<Vec<(String, Vec<String>)>> {
    let mut found = Vec::new();
    for playlist in apple
        .run(&FetchAllPlaylistsRequest { call_id: None })
        .await?
        .playlists
    {
        let page = apple.run(&playlist_page(playlist.id)).await?;
        found.push((
            playlist.name,
            page.entries
                .into_iter()
                .map(|entry| entry.track_uuid)
                .collect(),
        ));
    }
    found.sort();
    Ok(found)
}

fn named_songs(list: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
    list.iter()
        .map(|(name, songs)| {
            (
                (*name).to_owned(),
                songs.iter().map(|song| (*song).to_owned()).collect(),
            )
        })
        .collect()
}

/// Every playlist of Apple's, as the sign-in imported it.
fn imported_playlists() -> Vec<(String, Vec<String>)> {
    named_songs(&[
        ("Evening Jazz", &["i.s2", "i.s1"]),
        ("Favourite Songs", &["i.s2"]),
        ("Road Trip", &["i.s1", "i.s3"]),
        ("Sunday", &["i.s3", "i.s2"]),
    ])
}

/// How often the songs of the subscribed playlist were read, which only an import reads: the
/// song list joins the user's own playlists only.
fn import_reads(apple: &OpenApple) -> usize {
    apple
        .fake
        .count("/v1/me/library/playlists/p.sub1/tracks")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_first_sign_in_imports_every_playlist_of_apples_once() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), with_playlists()).await?;
    let folder = records_of(&apple, apple.record);
    // Every playlist, with its songs in order, the Favourite Songs and the one subscribed to
    // included.
    assert_eq!(rune_playlist_songs(&apple).await?, imported_playlists());
    assert!(folder.join("playlists.imported").exists());
    // Each song is kept with what was known of it, to name it once Apple drops it.
    let written = std::fs::read_to_string(folder.join("playlists.json"))?;
    for kept in [
        "\"imported_from\": \"p.mine2\"",
        "\"catalog_id\": \"ci.s2\"",
        "\"title\": \"Second Wind\"",
        "\"album\": \"Dawn Album\"",
    ] {
        assert!(written.contains(kept), "{kept} in {written}");
    }
    assert!(apple.fake.writes.lock().unwrap().is_empty());

    // Another sign-in, and a binding made afresh as after a restart, import nothing again, and
    // do not even read Apple's playlists for it.
    let reads = import_reads(&apple);
    assert_eq!(reads, 1);
    sign_in_again(&apple, USER_TOKEN).await?;
    import_settled(&apple.binding).await?;
    let (_ui, binding) = bind_again(&apple).await?;
    import_settled(&binding).await?;
    assert_eq!(import_reads(&apple), reads);
    assert_eq!(rune_playlist_songs(&apple).await?, imported_playlists());
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_playlist_apple_refuses_alone_costs_no_first_song_list() -> Result<()> {
    const SUNDAY: &str = "/v1/me/library/playlists/p.mine2/tracks";
    let dir = tempfile::tempdir()?;
    // Apple refuses the songs of "Sunday", though the sign-in is fine.
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            respond: Some((SUNDAY, 403, r#"{"errors":[{"status":"403"}]}"#)),
            import_unfinished: true,
            ..with_playlists()
        },
    )
    .await?;
    let songs = apple.binding.songs().await?;
    assert_eq!(songs.len(), 3);
    assert!(apple.fake.count(SUNDAY) > 0);
    assert!(apple.binding.is_signed_in());
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn playlists_apple_fails_to_list_cost_no_first_song_list() -> Result<()> {
    const PLAYLISTS: &str = "/v1/me/library/playlists";
    let dir = tempfile::tempdir()?;
    // Apple fails to list the playlists, though it lists the library's songs.
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            respond: Some((PLAYLISTS, 500, r#"{"errors":[{"status":"500"}]}"#)),
            import_unfinished: true,
            ..with_playlists()
        },
    )
    .await?;
    let songs = apple.binding.songs().await?;
    assert_eq!(songs.len(), 3);
    assert!(apple.fake.count(PLAYLISTS) > 0);
    assert!(apple.binding.is_signed_in());
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_playlist_read_before_another_sign_in_is_read_again_for_it() -> Result<()> {
    const JAZZ: &str = "/v1/me/library/playlists/p.sub1/tracks";
    let dir = tempfile::tempdir()?;
    // Apple holds the songs of "Evening Jazz" at the import of the first sign-in.
    let release = Arc::new(tokio::sync::Notify::new());
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            hold: Some((JAZZ, release.clone())),
            import_unfinished: true,
            ..with_playlists()
        },
    )
    .await?;
    fake_asked(&apple, JAZZ, 1).await?;
    // Another sign-in comes meanwhile: what was read for the first is not written for it, and
    // the import it starts reads the playlist again.
    sign_in_again(&apple, APPLE_OTHER_TOKEN).await?;
    release.notify_one();
    import_ended(&apple, apple.record).await?;
    assert_eq!(apple.fake.count(JAZZ), 2);
    assert_eq!(rune_playlist_songs(&apple).await?, imported_playlists());
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_import_cut_short_goes_on_at_the_next_sign_in_beside_rune_playlists() -> Result<()> {
    let dir = tempfile::tempdir()?;
    // Apple fails the songs of "Evening Jazz" once, at the import of the first sign-in.
    let apple = open_apple_with(
        dir.path(),
        AppleOptions {
            fail_once: Some("/v1/me/library/playlists/p.sub1/tracks"),
            import_unfinished: true,
            ..with_playlists()
        },
    )
    .await?;
    let folder = records_of(&apple, apple.record);
    // The import ends with the ids of the account, which come after the likes.
    wait_until("the first import to end", || async {
        Ok(folder.join("account.json").exists().then_some(()))
    })
    .await?;
    let marker = folder.join("playlists.imported");
    assert_eq!(
        rune_playlist_songs(&apple).await?,
        named_songs(&[
            ("Favourite Songs", &["i.s2"]),
            ("Road Trip", &["i.s1", "i.s3"]),
            ("Sunday", &["i.s3", "i.s2"]),
        ])
    );
    assert!(!marker.exists());
    // A Rune playlist made meanwhile, as one made before this device imported anything.
    let mine = make_rune_playlist(&apple, "Mine", "Mood").await?;
    add_song(&apple, mine.id, "i.s2", None).await?;

    // The next sign-in imports what is missing, beside it, and nothing twice.
    sign_in_again(&apple, USER_TOKEN).await?;
    wait_until("the playlists to be imported", || async {
        Ok(marker.exists().then_some(()))
    })
    .await?;
    let mut all = imported_playlists();
    all.push(("Mine".to_owned(), vec!["i.s2".to_owned()]));
    all.sort();
    assert_eq!(rune_playlist_songs(&apple).await?, all);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn playlists_the_user_cannot_change_are_editable_playlists_of_rune_and_no_mixes() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), with_playlists()).await?;
    let album = apple.binding.intern(AppleIdKind::Album, "l.alb1")?;
    create_mix(&apple, "Mine", vec![query("lib::album", album)]).await?;
    // Only Rune's own mix is listed: the Mixes page has no group for Apple's playlists, and the
    // home and the menus hold none either.
    assert_eq!(mix_summary(&apple).await?, [("Favorite".to_owned(), 1)]);
    let page = mix_groups(&apple, &["Apple Music"]).await?;
    assert!(page[0].collections.is_empty());
    let home = apple.run(&home_mixes(LibraryHomeListMode::Forward)).await?;
    assert_eq!(
        home.result[0]
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.readonly))
            .collect::<Vec<_>>(),
        [("Mine", false)]
    );
    assert_eq!(
        rune_mixes(&apple)
            .await?
            .iter()
            .map(|mix| mix.name.as_str())
            .collect::<Vec<_>>(),
        ["Mine"]
    );

    // The one subscribed to and the Favourite Songs are playlists of Rune's, with their songs
    // in order, that can be changed like any other, renamed within their group too, and nothing
    // is written to Apple.
    let jazz = playlist_named(&apple, "Evening Jazz").await?;
    add_song(&apple, jazz, "i.s3", None).await?;
    apple
        .run(&UpdatePlaylistRequest {
            playlist_id: jazz,
            name: "Late Jazz".into(),
            group: "Apple Music".into(),
            call_id: None,
        })
        .await?;
    let favourites = playlist_named(&apple, "Favourite Songs").await?;
    add_song(&apple, favourites, "i.s1", Some(0)).await?;
    assert_eq!(
        rune_playlist_songs(&apple).await?,
        named_songs(&[
            ("Favourite Songs", &["i.s1", "i.s2"]),
            ("Late Jazz", &["i.s2", "i.s1", "i.s3"]),
            ("Road Trip", &["i.s1", "i.s3"]),
            ("Sunday", &["i.s3", "i.s2"]),
        ])
    );
    assert!(apple.fake.writes.lock().unwrap().is_empty());
    apple.finish().await
}

/// The files the Apple Music library of `record` keeps on this device beside Apple's: the marker
/// of the import of the playlists, the playlists, the ids of the account, the mixes, the liked
/// songs and the marker of the import of the likes.
fn rune_files(apple: &OpenApple, record: LibraryRecordId) -> [std::path::PathBuf; 6] {
    let folder = records_of(apple, record);
    [
        "playlists.imported",
        "playlists.json",
        "account.json",
        "mixes.json",
        "likes.json",
        "likes.imported",
    ]
    .map(|name| folder.join(name))
}

/// What the files of `record` hold, as `rune_files` lists them; none for a file it lacks.
fn rune_file_bytes(apple: &OpenApple, record: LibraryRecordId) -> Vec<Option<String>> {
    rune_files(apple, record)
        .iter()
        .map(|file| std::fs::read_to_string(file).ok())
        .collect()
}

/// Removes the Apple Music library `record` as the settings do, or as a reset does with
/// `delete_device_data`.
async fn remove_apple(
    apple: &OpenApple,
    record: LibraryRecordId,
    delete_device_data: bool,
) -> Result<()> {
    use crate::messages::RemoveLibraryRecordRequest;

    open_own_library(apple).await?;
    let remove = RemoveLibraryRecordRequest {
        record_id: record.0.to_string(),
        call_id: None,
        delete_device_data,
    };
    <RemoveLibraryRecordRequest as crate::Signal>::handle(
        &remove,
        apple.node.clone(),
        None,
        &remove,
    )
    .await?
    .context("the record's removal")?;
    Ok(())
}

/// Adds the Apple Music library again, as the sign-in does: a new record, opened once the import
/// its sign-in started ended.
async fn add_apple_again(apple: OpenApple) -> Result<OpenApple> {
    use crate::messages::{RegisterServiceLibraryRequest, ServiceSignIn, StreamingService};
    use crate::utils::register_service_library_on_node;

    let registered = register_service_library_on_node(
        &RegisterServiceLibraryRequest {
            service: StreamingService::AppleMusic,
            alias: "Apple Music".into(),
            sign_in: ServiceSignIn::Platform {
                user_token: USER_TOKEN.into(),
                developer_token: Some(DEVELOPER_TOKEN.into()),
            },
            call_id: None,
        },
        &apple.node,
    )
    .await?;
    let record = LibraryRecordId(
        registered
            .record
            .context("registered record")?
            .record_id
            .parse()?,
    );
    assert_ne!(record, apple.record);
    let binding = apple
        .node
        .services
        .apple_music_binding(record)
        .context("the binding of the library added again")?;
    apple
        .node
        .open_service_catalog(Some(record), binding.clone())
        .await?;
    let ui = apple
        .node
        .current_ui()
        .await
        .context("UI params of the open library")?;
    let apple = OpenApple {
        binding,
        record,
        ui,
        ..apple
    };
    import_ended(&apple, record).await?;
    Ok(apple)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_removed_library_comes_back_for_its_own_account_only() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), with_playlists()).await?;
    // Apple's playlists were imported; the user made a playlist, a mix and a like of their own.
    let night = make_rune_playlist(&apple, "Night Drive", "Mood").await?;
    add_song(&apple, night.id, "i.s2", None).await?;
    let album = apple.binding.intern(AppleIdKind::Album, "l.alb1")?;
    let mine = create_mix(&apple, "Mine", vec![query("lib::album", album)]).await?;
    let playlists = rune_playlist_songs(&apple).await?;
    assert_eq!(playlists.len(), 5);
    let mix_songs = mix_titles(&apple, stored_queries(&apple, mine.id).await?).await?;
    assert!(!mix_songs.is_empty());
    apple.run(&like(&apple, "i.s2", true)).await?;
    // Some ids of the account's library are kept beside them.
    let account = std::fs::read_to_string(&rune_files(&apple, apple.record)[2])?;
    for id in ["\"i.s1\"", "\"p.mine2\""] {
        assert!(account.contains(id), "{id} in {account}");
    }

    // Removing the library keeps them on this device as they are, and nothing else of it.
    let first = apple.record;
    remove_apple(&apple, first, false).await?;
    let kept = rune_file_bytes(&apple, first);
    assert!(kept.iter().all(Option::is_some));
    let left: Vec<_> = std::fs::read_dir(records_of(&apple, first).parent().context("folder")?)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<_>>()?;
    assert_eq!(left, ["rune"]);

    // The same account added again takes them over, and imports nothing again, the likes
    // included: Apple's ratings are not read again.
    let reads = import_reads(&apple);
    let ratings = apple.fake.rating_reads.load(Ordering::SeqCst);
    let apple = add_apple_again(apple).await?;
    let second = apple.record;
    assert_eq!(import_reads(&apple), reads);
    assert_eq!(apple.fake.rating_reads.load(Ordering::SeqCst), ratings);
    assert!(is_liked(&apple, "i.s2").await?);
    assert!(!records_of(&apple, first).parent().context("folder")?.exists());
    assert_eq!(rune_file_bytes(&apple, second), kept);
    // It lists the playlists and mixes, which play what they did.
    assert_eq!(rune_playlist_songs(&apple).await?, playlists);
    let night = playlist_named(&apple, "Night Drive").await?;
    assert_eq!(
        mix_tracks(apple.run(&mix("lib::playlist", night)).await?),
        ["i.s2"]
    );
    let mixes = rune_mixes(&apple).await?;
    assert_eq!(
        mixes
            .iter()
            .map(|mix| mix.name.as_str())
            .collect::<Vec<_>>(),
        ["Mine"]
    );
    let queries = stored_queries(&apple, mixes[0].id).await?;
    assert_eq!(query_keys(&apple.binding, &queries)?, ["l.alb1"]);
    assert_eq!(mix_titles(&apple, queries).await?, mix_songs);

    // Another account added after it was removed takes nothing over: it imports its own
    // playlists, and what the first account left stays as it was.
    remove_apple(&apple, second, false).await?;
    let kept = rune_file_bytes(&apple, second);
    apple.fake.other_account.store(true, Ordering::SeqCst);
    let apple = add_apple_again(apple).await?;
    let other = apple.record;
    assert_eq!(rune_file_bytes(&apple, second), kept);
    assert_eq!(
        rune_playlist_songs(&apple).await?,
        named_songs(&[
            ("Evening Jazz", &["i.b-s2", "i.b-s1"]),
            ("Favourite Songs", &["i.b-s2"]),
            ("Road Trip", &["i.b-s1", "i.b-s3"]),
            ("Sunday", &["i.b-s3", "i.b-s2"]),
        ])
    );
    assert!(rune_mixes(&apple).await?.is_empty());
    // Nor does it take the likes of the first account.
    assert!(!is_liked(&apple, "i.b-s2").await?);
    apple.run(&like(&apple, "i.b-s1", true)).await?;
    let account = std::fs::read_to_string(&rune_files(&apple, other)[2])?;
    assert!(account.contains("\"i.b-s1\""), "{account}");
    assert!(!account.contains("\"i.s1\""), "{account}");

    // Going back to the first account takes its own over again; the other's stay.
    remove_apple(&apple, other, false).await?;
    let others = rune_file_bytes(&apple, other);
    apple.fake.other_account.store(false, Ordering::SeqCst);
    let apple = add_apple_again(apple).await?;
    assert!(rune_file_bytes(&apple, second).iter().all(Option::is_none));
    assert_eq!(rune_file_bytes(&apple, other), others);
    assert_eq!(rune_playlist_songs(&apple).await?, playlists);
    assert_eq!(rune_mixes(&apple).await?.len(), 1);
    assert!(is_liked(&apple, "i.s2").await?);
    assert!(!is_liked(&apple, "i.s1").await?);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn whose_songs_records_name_is_asked_of_the_account_signed_in_now() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    apple.binding.songs().await?;
    let s1 = ["i.s1".to_owned()];
    assert!(
        apple
            .binding
            .known_songs()
            .is_some_and(|songs| songs.get("i.s1").is_some())
    );
    assert_eq!(apple.binding.holds_any_song(&s1).await, Ok(true));
    // Under another account's tokens the song list kept is still the first account's, but the
    // library signed in now does not hold the song.
    apple.fake.other_account.store(true, Ordering::SeqCst);
    assert_eq!(apple.binding.holds_any_song(&s1).await, Ok(false));
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reset_deletes_them_and_stores_without_account_ids_go_by_the_ids_they_hold() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), with_playlists()).await?;
    let mine = make_rune_playlist(&apple, "Mine", "Mood").await?;
    add_song(&apple, mine.id, "i.s2", None).await?;
    create_mix(&apple, "Mix", vec![query("lib::playlist", mine.id)]).await?;
    apple.run(&like(&apple, "i.s1", true)).await?;
    let first = apple.record;
    assert!(rune_file_bytes(&apple, first).iter().all(Option::is_some));

    // A reset deletes them with the record.
    remove_apple(&apple, first, true).await?;
    assert!(!records_of(&apple, first).parent().context("folder")?.exists());

    // Records of removed libraries without the ids of their account: one whose songs the
    // library holds, and one, written later, that names no song of a library at all.
    let (holds, names_none) = (
        LibraryRecordId(uuid::Uuid::new_v4()),
        LibraryRecordId(uuid::Uuid::new_v4()),
    );
    let write = |record: LibraryRecordId, track: &str| -> Result<()> {
        let files = rune_files(&apple, record);
        std::fs::create_dir_all(files[1].parent().context("the folder")?)?;
        std::fs::write(&files[0], "")?;
        std::fs::write(
            &files[1],
            format!(
                r#"{{"format": 1, "playlists": [{{"id": "{}", "name": "Kept {track}",
                    "group": "Mood", "entries": [{{"occurrence": "o1", "track": "{track}"}}]}}]}}"#,
                record.0
            ),
        )?;
        Ok(())
    };
    write(holds, "i.s3")?;
    std::thread::sleep(std::time::Duration::from_millis(20));
    write(names_none, "catalog:900")?;
    let untouched = rune_file_bytes(&apple, names_none);

    // The library added again takes over those whose songs it holds, with their marker, so it
    // imports no playlist; the others stay.
    let reads = import_reads(&apple);
    let apple = add_apple_again(apple).await?;
    assert_eq!(import_reads(&apple), reads);
    assert_eq!(
        rune_playlist_songs(&apple).await?,
        named_songs(&[("Kept i.s3", &["i.s3"])])
    );
    assert!(!records_of(&apple, holds).parent().context("folder")?.exists());
    assert_eq!(rune_file_bytes(&apple, names_none), untouched);
    // Its account's ids are kept beside them from then on.
    let account = std::fs::read_to_string(&rune_files(&apple, apple.record)[2])?;
    assert!(account.contains("\"i.s3\""), "{account}");
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reset_deletes_the_records_a_library_removed_before_left() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    make_rune_playlist(&apple, "Mine", "Mood").await?;
    let first = apple.record;
    remove_apple(&apple, first, false).await?;
    assert!(records_of(&apple, first).is_dir());
    // Another account's library takes nothing over; a reset removes it, and what the first left.
    apple.fake.other_account.store(true, Ordering::SeqCst);
    let apple = add_apple_again(apple).await?;
    let other = apple.record;
    assert!(records_of(&apple, first).is_dir());
    remove_apple(&apple, other, true).await?;
    for record in [first, other] {
        assert!(!records_of(&apple, record).parent().context("folder")?.exists());
    }
    apple.finish().await
}

// What Rune's playlists, mixes and likes add to the library ---------------------------------

/// Every album of the Albums page, by name, with its id.
async fn all_albums(apple: &OpenApple) -> Result<Vec<(String, i32)>> {
    let titles: Vec<String> = apple
        .run(&FetchCollectionGroupSummaryRequest {
            call_id: None,
            collection_type: CollectionType::Album,
        })
        .await?
        .groups
        .into_iter()
        .map(|group| group.group_title)
        .collect();
    let mut listed: Vec<(String, i32)> = apple
        .run(&FetchCollectionGroupsRequest {
            call_id: None,
            collection_type: CollectionType::Album,
            bake_cover_arts: false,
            group_titles: titles,
        })
        .await?
        .groups
        .into_iter()
        .flat_map(|group| group.collections)
        .map(|collection| (collection.name, collection.id))
        .collect();
    listed.sort();
    Ok(listed)
}

/// The titles of the Tracks page in order, and the count it shows.
async fn all_tracks(apple: &OpenApple) -> Result<(Vec<String>, i32)> {
    let count = apple
        .run(&GetMediaFilesCountRequest { call_id: None })
        .await?
        .count;
    let titles = apple
        .run(&FetchMediaFilesRequest {
            call_id: None,
            cursor: 0,
            page_size: 50,
            bake_cover_arts: false,
        })
        .await?
        .media_files
        .into_iter()
        .map(|file| file.title)
        .collect();
    Ok((titles, count))
}

/// Waits for the Albums page to list `name`, which a read in the background adds, and answers
/// its id.
async fn album_listed(apple: &OpenApple, name: &str) -> Result<i32> {
    wait_until("the album to be listed", || async {
        let listed = all_albums(apple).await?;
        Ok(listed
            .iter()
            .find(|(listed, _)| listed == name)
            .map(|(_, id)| *id))
    })
    .await
}

/// The titles of the songs of an album's page.
async fn album_titles(apple: &OpenApple, album: i32) -> Result<Vec<String>> {
    Ok(apple
        .run(&mix("lib::album", album))
        .await?
        .files
        .into_iter()
        .map(|file| file.title)
        .collect())
}

fn titled(titles: &[&str], count: i32) -> (Vec<String>, i32) {
    (titles.iter().map(|title| (*title).to_owned()).collect(), count)
}

/// What the library keeps of the catalog for Rune's records, as written beside its lists.
fn held_file(apple: &OpenApple) -> std::path::PathBuf {
    lists_folder(Path::new(&*apple.node.config_path), apple.record.0).join("held.json")
}

/// Waits until what the library keeps of the catalog for Rune's records names every one of
/// `keys` on disk.
async fn held_on_disk(apple: &OpenApple, keys: &[&str]) -> Result<()> {
    let file = held_file(apple);
    wait_until("what was read to be kept on disk", || async {
        let held = std::fs::read_to_string(&file).unwrap_or_default();
        let kept = keys.iter().all(|key| held.contains(&format!("\"{key}\"")));
        Ok(kept.then_some(()))
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_catalog_album_in_a_rune_mix_is_in_the_library_until_no_mix_names_it() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let catalog = apple.binding.intern(AppleIdKind::Album, "catalog:1600")?;
    let made = create_mix(&apple, "Remixes", vec![query("lib::album", catalog)]).await?;

    // The album joins the Albums page as an album of the library, apart from the catalog's.
    let album = album_listed(&apple, "Dawn Remixes").await?;
    assert_eq!(
        apple.binding.resolve(AppleIdKind::Album, album)?,
        "rune-album:1600"
    );
    assert_eq!(
        names(&all_albums(&apple).await?),
        ["Dawn Album", "Dawn Remixes", "Night Album"]
    );
    assert_eq!(
        collection_names(&apple, CollectionType::Album, &[album]).await?,
        ["Dawn Remixes"]
    );
    // Its songs follow the library's on the Tracks page, and its page holds them all.
    assert_eq!(
        all_tracks(&apple).await?,
        titled(
            &["First Light", "Second Wind", "Third Road", "Remix One", "Remix Two"],
            5
        )
    );
    assert_eq!(album_titles(&apple, album).await?, ["Remix One", "Remix Two"]);
    // A song of it is on that album.
    let remix = apple.binding.intern_track("catalog:1601")?;
    let parsed = apple
        .run(&FetchParsedMediaFileRequest {
            call_id: None,
            id: remix,
        })
        .await?;
    assert_eq!(
        (parsed.album.id, parsed.album.name.as_str()),
        (album, "Dawn Remixes")
    );
    // Nothing went into Apple's library, and the pages list it again without reading it again.
    assert!(apple.fake.writes.lock().unwrap().is_empty());
    // The facts the artist pages place the album by are read in the background once it is listed.
    wait_until("the album to be classified", || async {
        let seen = apple.fake.seen.lock().unwrap();
        Ok(seen
            .iter()
            .any(|(target, _)| target.starts_with("/v1/catalog/it/albums?ids=1600"))
            .then_some(()))
    })
    .await?;
    let reads = apple.fake.total();
    for _ in 0..2 {
        all_albums(&apple).await?;
        all_tracks(&apple).await?;
    }
    assert_eq!(apple.fake.total(), reads);
    // Its artist is an artist of the library, as the artists of its songs are.
    assert!(names(&all_artists(&apple).await?).contains(&"Dawn Singer"));

    // Once no mix names it, the library does not hold it.
    apple
        .run(&RemoveMixRequest {
            mix_id: made.id,
            call_id: None,
        })
        .await?;
    assert_eq!(names(&all_albums(&apple).await?), ["Dawn Album", "Night Album"]);
    assert_eq!(all_tracks(&apple).await?.1, 3);
    assert!(!names(&all_artists(&apple).await?).contains(&"Dawn Singer"));
    // Its page lists nothing, and reads nothing from Apple.
    let reads = apple.fake.total();
    assert!(album_titles(&apple, album).await?.is_empty());
    assert_eq!(apple.fake.total(), reads);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_was_read_for_a_mix_stays_while_the_file_of_the_mixes_cannot_be_read() -> Result<()> {
    use crate::service::rune_mixes::MIXES_FILE;

    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let catalog = apple.binding.intern(AppleIdKind::Album, "catalog:1600")?;
    create_mix(&apple, "Remixes", vec![query("lib::album", catalog)]).await?;
    album_listed(&apple, "Dawn Remixes").await?;
    held_on_disk(&apple, &["1600"]).await?;
    // The file of the mixes cannot be read once the app starts again, offline. No mix names
    // the album then, so it is not listed.
    let mixes = records_of(&apple, apple.record).join(MIXES_FILE);
    let kept = std::fs::read(&mixes)?;
    std::fs::write(&mixes, b"not a list of mixes")?;
    let restarted = restart_apple(apple).await?;
    let (_ui, binding) = restarted.open().await?;
    let album_names = async || -> Result<Vec<String>> {
        Ok(binding
            .albums()
            .await?
            .iter()
            .filter_map(|album| album.attributes.as_ref()?.name.clone())
            .collect())
    };
    assert!(!album_names().await?.contains(&"Dawn Remixes".to_owned()));
    // Once it can be read again, the album is listed from what was read for it, which a library
    // that is not signed in cannot read again.
    std::fs::write(&mixes, kept)?;
    let listed = album_names().await?;
    assert!(listed.contains(&"Dawn Remixes".to_owned()), "{listed:?}");
    restarted.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_catalog_song_in_a_rune_playlist_brings_its_album_holding_only_it() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let mine = make_rune_playlist(&apple, "Mine", "Favorite").await?;
    let occurrence = add_song(&apple, mine.id, "catalog:1601", None).await?;

    // The album holds the one song, as the Apple library holds an album some of whose songs
    // it holds; the catalog's album still holds both.
    let album = album_listed(&apple, "Dawn Remixes").await?;
    assert_eq!(album_titles(&apple, album).await?, ["Remix One"]);
    let catalog = apple.binding.intern(AppleIdKind::Album, "catalog:1600")?;
    assert_eq!(
        album_titles(&apple, catalog).await?,
        ["Remix One", "Remix Two"]
    );
    assert_eq!(
        all_tracks(&apple).await?,
        titled(&["First Light", "Second Wind", "Third Road", "Remix One"], 4)
    );
    assert!(apple.fake.writes.lock().unwrap().is_empty());

    // Taken out of the playlist, it leaves the library with its album.
    apple
        .run(&RemoveItemFromPlaylistRequest {
            playlist_id: mine.id,
            occurrence_id: occurrence,
            call_id: None,
        })
        .await?;
    assert_eq!(names(&all_albums(&apple).await?), ["Dawn Album", "Night Album"]);
    assert_eq!(all_tracks(&apple).await?.1, 3);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_the_library_holds_already_is_not_listed_again_for_a_mix_or_a_playlist() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    // The catalog's "Dawn Album" holds the catalog song the library's "First Light" is a copy
    // of.
    let catalog = apple.binding.intern(AppleIdKind::Album, "catalog:1500")?;
    create_mix(&apple, "Dawn", vec![query("lib::album", catalog)]).await?;
    let mine = make_rune_playlist(&apple, "Mine", "Favorite").await?;
    add_song(&apple, mine.id, "catalog:ci.s1", None).await?;
    all_tracks(&apple).await?;
    held_on_disk(&apple, &["1500", "catalog:ci.s1"]).await?;

    assert_eq!(names(&all_albums(&apple).await?), ["Dawn Album", "Night Album"]);
    assert_eq!(
        all_tracks(&apple).await?,
        titled(&["First Light", "Second Wind", "Third Road"], 3)
    );
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_catalog_song_liked_in_rune_is_in_the_library_until_it_is_unliked() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (apple, heard) = open_apple_heard(dir.path()).await?;
    let told = told_settled(&heard).await;

    apple.run(&like(&apple, "catalog:1601", true)).await?;
    // The pages that show the library are told, and the song comes with its album, which holds
    // only it.
    told_after(&heard, told).await?;
    let album = album_listed(&apple, "Dawn Remixes").await?;
    assert_eq!(album_titles(&apple, album).await?, ["Remix One"]);
    assert_eq!(
        all_tracks(&apple).await?,
        titled(&["First Light", "Second Wind", "Third Road", "Remix One"], 4)
    );
    assert!(apple.fake.writes.lock().unwrap().is_empty());

    // Unliked, it leaves the library with its album.
    apple.run(&like(&apple, "catalog:1601", false)).await?;
    assert_eq!(names(&all_albums(&apple).await?), ["Dawn Album", "Night Album"]);
    assert_eq!(all_tracks(&apple).await?.1, 3);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_song_liked_and_in_a_playlist_stays_until_neither_holds_it() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let mine = make_rune_playlist(&apple, "Mine", "Favorite").await?;
    let occurrence = add_song(&apple, mine.id, "catalog:1601", None).await?;
    apple.run(&like(&apple, "catalog:1601", true)).await?;
    let album = album_listed(&apple, "Dawn Remixes").await?;

    apple
        .run(&RemoveItemFromPlaylistRequest {
            playlist_id: mine.id,
            occurrence_id: occurrence,
            call_id: None,
        })
        .await?;
    // The like still holds it.
    assert_eq!(album_titles(&apple, album).await?, ["Remix One"]);
    assert_eq!(all_tracks(&apple).await?.1, 4);

    apple.run(&like(&apple, "catalog:1601", false)).await?;
    assert_eq!(names(&all_albums(&apple).await?), ["Dawn Album", "Night Album"]);
    assert_eq!(all_tracks(&apple).await?.1, 3);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_catalog_songs_of_an_imported_playlist_are_read_together_and_kept_on_disk() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let apple = open_apple_with(dir.path(), with_playlists()).await?;
    // The playlist the sign-in imported from Apple's is a Rune playlist like any other: songs of
    // two albums and one Apple names no album for, beside songs of the library.
    let road_trip = playlist_named(&apple, "Road Trip").await?;
    for track in [
        "catalog:1601",
        "catalog:8001",
        "catalog:1602",
        "catalog:777",
        "i.s2",
    ] {
        add_song(&apple, road_trip, track, None).await?;
    }
    let songs_read = apple.fake.count("/v1/catalog/it/songs");

    let remixes = album_listed(&apple, "Dawn Remixes").await?;
    let elsewhere = album_listed(&apple, "Elsewhere").await?;
    // The songs were read together, with their albums, and no album on its own.
    assert_eq!(apple.fake.count("/v1/catalog/it/songs") - songs_read, 1);
    assert_eq!(apple.fake.count("/v1/catalog/it/albums/"), 0);
    assert_eq!(
        names(&all_albums(&apple).await?),
        ["Dawn Album", "Dawn Remixes", "Elsewhere", "Night Album"]
    );
    assert_eq!(
        album_titles(&apple, remixes).await?,
        ["Remix One", "Remix Two"]
    );
    assert_eq!(album_titles(&apple, elsewhere).await?, ["Unheld Hit"]);
    assert_eq!(
        all_tracks(&apple).await?,
        titled(
            &[
                "First Light",
                "Second Wind",
                "Third Road",
                "Remix One",
                "Unheld Hit",
                "Remix Two",
                "Catalog Tune"
            ],
            7
        )
    );

    // What was read is kept beside the lists, so the library opened again, as after a restart,
    // lists it before its sign-in without asking Apple.
    held_on_disk(
        &apple,
        &["catalog:1601", "catalog:1602", "catalog:8001", "catalog:777"],
    )
    .await?;
    let restarted = restart_apple(apple).await?;
    let (_ui, binding) = restarted.open().await?;
    let asked = restarted.fake.total();
    let albums: Vec<String> = binding
        .albums()
        .await?
        .iter()
        .filter_map(|album| album.attributes.as_ref()?.name.clone())
        .collect();
    assert_eq!(
        albums,
        ["Dawn Album", "Night Album", "Dawn Remixes", "Elsewhere"]
    );
    assert_eq!(binding.songs().await?.len(), 7);
    assert_eq!(restarted.fake.total(), asked);
    restarted.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_burst_of_songs_added_to_a_playlist_is_kept_and_tells_the_ui() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (apple, heard) = open_apple_heard(dir.path()).await?;
    let mine = make_rune_playlist(&apple, "Mine", "Favorite").await?;
    let told = told_settled(&heard).await;

    for _ in 0..10 {
        for song in ["i.s1", "i.s2", "i.s3"] {
            add_song(&apple, mine.id, song, None).await?;
        }
    }
    // The UI is told once the changes come to rest. How often depends on how fast the songs are
    // written, so the binding's own tests tell a burst apart on a clock they move.
    told_after(&heard, told).await?;
    let page = apple.run(&playlist_page(mine.id)).await?;
    assert_eq!(page.entries.len(), 30);
    apple.finish().await
}

// Mixes follow Apple -------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_catalog_playlist_a_mix_names_holds_its_songs_as_apple_changes_them() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (apple, heard) = open_apple_heard(dir.path()).await?;
    let picks = apple
        .binding
        .intern(AppleIdKind::Playlist, "catalog:pl.cat1")?;
    let made = create_mix(&apple, "Picks", vec![query("lib::playlist", picks)]).await?;

    // Its songs are the library's.
    wait_until("the songs of the playlist to be listed", || async {
        Ok((all_tracks(&apple).await?.1 == 5).then_some(()))
    })
    .await?;
    assert_eq!(
        all_tracks(&apple).await?.0,
        [
            "First Light",
            "Second Wind",
            "Third Road",
            "Dawn Chorus",
            "Catalog Tune"
        ]
    );

    // Apple changes the playlist: once a mix of it reads it again, the library follows, and the
    // pages that show it are told.
    let changed = json!({"data": [apple.fake.catalog_song("1601")]});
    *apple.fake.respond_path.lock().unwrap() = Some((
        "/v1/catalog/it/playlists/pl.cat1/tracks".into(),
        200,
        changed.to_string(),
    ));
    let told = told_settled(&heard).await;
    assert_eq!(
        mix_titles(&apple, vec![query("lib::playlist", picks)]).await?,
        ["Remix One"]
    );
    told_after(&heard, told).await?;
    assert_eq!(
        all_tracks(&apple).await?.0,
        ["First Light", "Second Wind", "Third Road", "Remix One"]
    );
    let album = album_listed(&apple, "Dawn Remixes").await?;
    assert_eq!(album_titles(&apple, album).await?, ["Remix One"]);

    // Once no mix names it, its songs leave the library.
    apple
        .run(&RemoveMixRequest {
            mix_id: made.id,
            call_id: None,
        })
        .await?;
    assert_eq!(all_tracks(&apple).await?.1, 3);
    assert_eq!(names(&all_albums(&apple).await?), ["Dawn Album", "Night Album"]);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_song_apple_lacked_under_the_tokens_before_is_asked_for_again_under_new_ones()
-> Result<()> {
    const GONE: &str = "catalog:404404";
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let asked = || {
        apple
            .fake
            .seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(target, _)| target.contains("404404"))
            .count()
    };
    // The queue's lookup asks Apple for a song it does not have, and Apple holds the answer.
    let release = Arc::new(tokio::sync::Notify::new());
    *apple.fake.hold_once.lock().unwrap() = Some(("/v1/catalog/it/songs".into(), release.clone()));
    let lookup = apple.binding.read_unknown_tracks([GONE.to_owned()]);
    let switch = async {
        wait_until("the song to be asked for", || async {
            Ok((asked() == 1).then_some(()))
        })
        .await?;
        // Other tokens sign in meanwhile, which may be another account's.
        apple
            .binding
            .sign_in(Tokens::new(DEVELOPER_TOKEN, APPLE_OTHER_TOKEN).context("tokens")?);
        release.notify_one();
        Ok::<_, anyhow::Error>(())
    };
    let (found, switch) = tokio::join!(lookup, switch);
    switch?;
    assert!(found.is_empty());
    // What Apple lacked under the tokens before is asked for again under the new ones, once.
    assert!(!apple.binding.known_missing(GONE));
    apple.binding.read_unknown_tracks([GONE.to_owned()]).await;
    assert_eq!(asked(), 2);
    assert!(apple.binding.known_missing(GONE));
    apple.binding.read_unknown_tracks([GONE.to_owned()]).await;
    assert_eq!(asked(), 2);
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_song_a_read_before_a_sign_out_asked_for_is_read_again_after_it() -> Result<()> {
    const SONGS: &str = "/v1/catalog/it/songs";
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let mine = make_rune_playlist(&apple, "Mine", "Mood").await?;
    add_song(&apple, mine.id, "catalog:900", None).await?;
    // The library reads the song in the background, and Apple holds the answer.
    let release = Arc::new(tokio::sync::Notify::new());
    *apple.fake.hold_once.lock().unwrap() = Some((SONGS.into(), release.clone()));
    let reads = apple.fake.count(SONGS);
    all_tracks(&apple).await?;
    fake_asked(&apple, SONGS, reads + 1).await?;
    // The account signs out and in again meanwhile, and the library is listed under it.
    apple.binding.sign_out();
    apple
        .binding
        .sign_in(Tokens::new(DEVELOPER_TOKEN, USER_TOKEN).context("tokens")?);
    all_tracks(&apple).await?;
    // What the read begun before brings is not kept, so the song is read again once it ends,
    // with nothing listed meanwhile.
    release.notify_one();
    fake_asked(&apple, SONGS, reads + 2).await?;
    wait_until("the song to be listed", || async {
        let titles = all_tracks(&apple).await?.0;
        Ok(titles.contains(&"Dawn Chorus".to_owned()).then_some(()))
    })
    .await?;
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_background_read_keeps_a_newer_read_of_a_playlist_a_mix_names() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    let picks = apple
        .binding
        .intern(AppleIdKind::Playlist, "catalog:pl.cat1")?;
    create_mix(&apple, "Picks", vec![query("lib::playlist", picks)]).await?;
    let mine = make_rune_playlist(&apple, "Mine", "Mood").await?;
    add_song(&apple, mine.id, "catalog:1602", None).await?;

    // The library reads in the background what the mix and the playlist name: the playlist,
    // then the song, whose answer Apple holds.
    let release = Arc::new(tokio::sync::Notify::new());
    *apple.fake.hold_once.lock().unwrap() = Some(("/v1/catalog/it/songs".into(), release.clone()));
    let songs = apple.fake.count("/v1/catalog/it/songs");
    all_tracks(&apple).await?;
    fake_asked(&apple, "/v1/catalog/it/songs", songs + 1).await?;

    // Meanwhile Apple changes the playlist, and the mix reads it again.
    let changed = json!({"data": [apple.fake.catalog_song("1601")]});
    *apple.fake.respond_path.lock().unwrap() = Some((
        "/v1/catalog/it/playlists/pl.cat1/tracks".into(),
        200,
        changed.to_string(),
    ));
    assert_eq!(
        mix_titles(&apple, vec![query("lib::playlist", picks)]).await?,
        ["Remix One"]
    );

    // Once the background read ends, the library still holds the playlist as it is now.
    release.notify_one();
    wait_until("the background read to end", || async {
        let titles = all_tracks(&apple).await?.0;
        Ok(titles.contains(&"Remix Two".to_owned()).then_some(titles))
    })
    .await?;
    let titles = all_tracks(&apple).await?.0;
    assert!(titles.contains(&"Remix One".to_owned()), "{titles:?}");
    assert!(!titles.contains(&"Dawn Chorus".to_owned()), "{titles:?}");
    apple.finish().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_library_artist_a_mix_names_brings_only_the_librarys_songs() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let apple = open_apple(dir.path()).await?;
    // What the library holds itself: an artist, an album, a song and a Rune playlist.
    let artist = artist_named(&all_artists(&apple).await?, "Artist One")?;
    let album = apple.binding.intern(AppleIdKind::Album, "l.alb2")?;
    let song = apple.binding.intern_track("i.s3")?;
    let mine = make_rune_playlist(&apple, "Mine", "Favorite").await?;
    add_song(&apple, mine.id, "i.s2", None).await?;
    let made = create_mix(
        &apple,
        "Mine",
        vec![
            query("lib::artist", artist),
            query("lib::album", album),
            query("lib::track", song),
            query("lib::playlist", mine.id),
        ],
    )
    .await?;
    assert_eq!(
        mix_titles(&apple, stored_queries(&apple, made.id).await?).await?,
        ["Third Road", "First Light", "Second Wind"]
    );

    // The library lists what it listed, and nothing is read from the catalog for it.
    assert_eq!(
        all_tracks(&apple).await?,
        titled(&["First Light", "Second Wind", "Third Road"], 3)
    );
    assert_eq!(names(&all_albums(&apple).await?), ["Dawn Album", "Night Album"]);
    tokio::time::sleep(CONTENT_CHANGED_DELAY).await;
    let read_for_it = |(target, _): &(String, bool)| {
        (target.starts_with("/v1/catalog/it/songs") && target.contains("include=albums"))
            || target.starts_with("/v1/catalog/it/albums/")
            || target.starts_with("/v1/catalog/it/playlists/")
    };
    assert!(!apple.fake.seen.lock().unwrap().iter().any(read_for_it));
    assert!(!held_file(&apple).exists());
    apple.finish().await
}

// Search and lyrics -------------------------------------------------------------------------

mod search {
    use super::*;
    include!("apple_music_search_tests.rs");
}

mod lyrics {
    use super::*;
    include!("apple_music_lyrics_tests.rs");
}

// The page of an artist ---------------------------------------------------------------------

mod artist_page {
    use super::*;
    include!("apple_music_artist_tests.rs");
}
