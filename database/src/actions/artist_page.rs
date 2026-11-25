//! The artist page of a local library: the releases an artist is on, sorted into sections, and
//! the artist's most listened songs.
//!
//! A local library holds no sections from a service, so they come from its catalog: the type
//! a release's tags give it, its compilation and live flags and its album artists. A release
//! whose tags give no type goes by its length.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet, hash_map::Entry};

use anyhow::Result;
use metadata::model::{ReleaseSecondaryType, ReleaseType};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, ExprTrait, FromQueryResult,
    QueryFilter,
    sea_query::{Alias, Order, Query, SelectStatement},
};
use uuid::Uuid;

use crate::entities::{
    media_file_likes, media_file_stats, media_files,
    music_metadata::{album_summaries, audio, credits, discs, values},
    portable_artists,
};
use crate::mix_operator::ArtistSection;

use super::browse::{self, Kind};
use super::mixes::{artist_uuid_condition, not_verified_duplicate};
use super::resolved_metadata::{
    CreditScope, column, ensure_tracks_current, queries::field, query_models, release_durations,
};

/// A release shorter than this, in milliseconds, can be a single or an EP.
const SHORT_RELEASE_MS: u64 = 30 * 60 * 1000;

/// The most tracks an EP has; an EP is listed with the singles.
const EP_MAX_TRACKS: usize = 6;

/// The role of an album artist's credit.
const PRIMARY_ARTIST: &str = "Primary Artist";

/// The field of a release's secondary types among the catalog's values.
const SECONDARY_TYPES: &str = "release_secondary_types";

/// The secondary types that decide a section.
const SECTION_FLAGS: [ReleaseSecondaryType; 2] = [
    ReleaseSecondaryType::Compilation,
    ReleaseSecondaryType::Live,
];

/// One song of a release.
#[derive(Clone, Debug, PartialEq)]
pub struct ReleaseTrack {
    pub file_id: i32,
    pub file_uuid: String,
    pub cover_art_uuid: Option<String>,
}

/// An album artist credit of a release.
#[derive(Clone, Debug, PartialEq)]
pub struct ReleaseArtist {
    /// The name as credited, which can name several artists, as "Ann & Bob" does.
    pub credited_as: String,
    /// The artist the catalog identified the credit as, if it identified one.
    pub artist_id: Option<String>,
}

/// The artist a page is of: its identity and its name.
#[derive(Clone, Debug, PartialEq)]
pub struct PageArtist {
    pub id: String,
    pub name: String,
}

/// What the artist page needs to know of a release, as the catalog resolves it.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// The id the library lists the release's album by.
    pub album_id: i32,
    pub name: String,
    /// The release date, which the catalog keeps as `YYYY`, `YYYY-MM` or `YYYY-MM-DD`.
    pub date: Option<String>,
    /// The type the tags give the release, if they give one.
    pub release_type: Option<ReleaseType>,
    /// Whether the tags flag the release as a compilation.
    pub compilation: bool,
    /// Whether the tags flag the release as live.
    pub live: bool,
    /// The album artists the release credits.
    pub album_artists: Vec<ReleaseArtist>,
    /// How many tracks the release has, each position counted once.
    pub track_count: usize,
    /// How long the release is in milliseconds, each position counted once.
    pub duration_ms: Option<u64>,
    /// The songs of the release in disc and track order.
    pub tracks: Vec<ReleaseTrack>,
}

impl Release {
    /// The year the release came out (see `year_of`).
    pub fn year(&self) -> Option<u32> {
        year_of(self.date.as_deref()?)
    }

    /// Whether the release is by the artist: an album artist credit is identified as the
    /// artist or names the artist, whole or as one of the artists it lists, or the release
    /// credits no album artist at all, so nothing says it is by others.
    fn is_by(&self, artist: &PageArtist) -> bool {
        let name = artist.name.trim().to_lowercase();
        self.album_artists.is_empty()
            || self.album_artists.iter().any(|credit| {
                credit.artist_id.as_deref() == Some(artist.id.as_str())
                    || credit.credited_as.trim().to_lowercase() == name
                    || metadata::artist::split_artists(&credit.credited_as)
                        .iter()
                        .any(|credited| credited.to_lowercase() == name)
            })
    }

    /// The section of `artist`'s page the release goes to. The first rule that holds decides:
    ///
    /// 1. A compilation flag: Compilations.
    /// 2. Album artists that are all someone else: Appears On.
    /// 3. A live flag: Live.
    /// 4. The release type: an EP or a single goes with the singles, an album with the
    ///    albums, and so do a broadcast and the other types, which have no section of their
    ///    own.
    /// 5. With no type, the release's length (see `section_by_size`).
    pub fn section_for(&self, artist: &PageArtist) -> ArtistSection {
        if self.compilation {
            ArtistSection::Compilations
        } else if !self.is_by(artist) {
            ArtistSection::AppearsOn
        } else if self.live {
            ArtistSection::Live
        } else {
            match self.release_type {
                Some(ReleaseType::Ep | ReleaseType::Single) => ArtistSection::Singles,
                Some(ReleaseType::Album | ReleaseType::Broadcast | ReleaseType::Other) => {
                    ArtistSection::Albums
                }
                None => section_by_size(self.track_count, self.duration_ms.unwrap_or(0)),
            }
        }
    }
}

/// The year of a release date as the catalog keeps it, from its first four digits.
pub fn year_of(date: &str) -> Option<u32> {
    date.get(..4)
        .filter(|year| year.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|year| year.parse().ok())
}

/// The title of the group of a section's albums whose year is unknown: the one the library's
/// grouped pages give what has no group of its own (see `generate_group_name`).
pub const UNDATED_GROUP: &str = "#";

/// A group of a page of the albums of a section of an artist's page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct YearGroup<T> {
    /// The year the albums came out, as digits, or `UNDATED_GROUP`.
    pub title: String,
    pub items: Vec<T>,
}

/// The page of at most `page_size` albums from `cursor` of a section whose albums are
/// `albums`, given in the section's order with their years, in groups by year: the newest year
/// first and the albums with no year last, each group in the section's order. A group a page
/// cuts goes on at the start of the next one, under the same title.
pub fn year_groups<T>(
    albums: impl IntoIterator<Item = (T, Option<u32>)>,
    cursor: usize,
    page_size: usize,
) -> Vec<YearGroup<T>> {
    let mut albums: Vec<(T, Option<u32>)> = albums.into_iter().collect();
    // The sort is stable, so each year keeps the section's order.
    albums.sort_by_key(|(_, year)| (year.is_none(), std::cmp::Reverse(*year)));
    let mut groups: Vec<YearGroup<T>> = Vec::new();
    for (item, year) in albums.into_iter().skip(cursor).take(page_size) {
        let title = year.map_or_else(|| UNDATED_GROUP.to_owned(), |year| year.to_string());
        match groups.last_mut() {
            Some(group) if group.title == title => group.items.push(item),
            _ => groups.push(YearGroup {
                title,
                items: vec![item],
            }),
        }
    }
    groups
}

/// Single (EPs are singles too) or album, by the number of tracks and the length in
/// milliseconds of a release whose tags give no type.
pub fn section_by_size(tracks: usize, duration_ms: u64) -> ArtistSection {
    if duration_ms >= SHORT_RELEASE_MS || tracks > EP_MAX_TRACKS {
        ArtistSection::Albums
    } else {
        ArtistSection::Singles
    }
}

/// Newest release first. The catalog's dates sort by date as text, a date that names no month
/// or day before the dates of its year or month that do. A release without a date goes after
/// the dated ones.
fn newest_first(a: &Release, b: &Release) -> Ordering {
    let by_date = match (&a.date, &b.date) {
        (Some(a), Some(b)) => b.cmp(a),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    };
    by_date
        .then_with(|| a.name.cmp(&b.name))
        .then_with(|| a.album_id.cmp(&b.album_id))
}

/// How many of a section's newest releases give their covers to its tile. The tile shows its
/// covers in circles that roll new ones in from its edges, so it needs more than the nine it
/// shows at once.
pub const SECTION_COVERS: usize = 16;

/// The sections of an artist's page that are not empty, and whether the artist has songs.
#[derive(Clone, Debug)]
pub struct ArtistPageSections {
    pub sections: Vec<ArtistSectionSummary>,
    pub has_songs: bool,
}

/// One section of an artist's page as the tile shows it.
#[derive(Clone, Debug)]
pub struct ArtistSectionSummary {
    pub section: ArtistSection,
    /// How many releases the section holds.
    pub releases: usize,
    /// One song with cover art for each of the newest releases, up to the limit asked for.
    pub cover_files: Vec<media_files::Model>,
}

/// An artist, the releases the artist has songs on, and the artist's songs.
struct ArtistCatalog {
    artist: PageArtist,
    /// Newest first.
    releases: Vec<Release>,
    files: HashMap<i32, media_files::Model>,
    /// The artist's own songs.
    songs: Vec<i32>,
    /// Selects the ids of the artist's own songs.
    song_query: SelectStatement,
}

/// One song of a release.
#[derive(FromQueryResult)]
struct TrackRow {
    album_id: String,
    file_id: i32,
}

/// A release as the album collection lists it, with its summary.
#[derive(FromQueryResult)]
struct ReleaseRow {
    id: i32,
    name: String,
    source_uuid: String,
    release_date: Option<String>,
    release_type: Option<String>,
    track_count: Option<i64>,
}

/// A secondary type a song's tags give its release.
#[derive(FromQueryResult)]
struct FlagRow {
    album_id: String,
    qualifier: String,
    value: String,
}

/// An album artist a song credits its release with.
#[derive(FromQueryResult)]
struct CreditRow {
    album_id: String,
    credited_as: String,
    artist_id: Option<String>,
}

/// Selects with each row the release its song is on, joining the song by `file_id`, and keeps
/// the rows of the releases `releases` selects, in release, disc and track order.
fn by_release(query: &mut SelectStatement, file_id: impl ColumnTrait, releases: SelectStatement) {
    query
        .column(discs::Column::AlbumId.as_column_ref())
        .inner_join(
            audio::Entity,
            column(audio::Column::FileId).eq(column(file_id)),
        )
        .inner_join(
            discs::Entity,
            column(discs::Column::Id).eq(column(audio::Column::DiscId)),
        )
        .and_where(column(discs::Column::AlbumId).in_subquery(releases))
        .order_by(discs::Column::DiscNumber.as_column_ref(), Order::Asc)
        .order_by_expr(audio::Column::TrackNumber.is_null(), Order::Asc)
        .order_by(audio::Column::TrackNumber.as_column_ref(), Order::Asc)
        .order_by(audio::Column::FileId.as_column_ref(), Order::Asc);
}

async fn load_catalog(db: &DatabaseConnection, artist_id: i32) -> Result<Option<ArtistCatalog>> {
    // Release summaries and logical tracks are brought up to date before they are read.
    ensure_tracks_current(db).await?;
    let Some(artist) = browse::by_id::<portable_artists::Model>(db, Kind::Artist, &[artist_id])
        .await?
        .into_iter()
        .next()
    else {
        return Ok(None);
    };
    let artist = PageArtist {
        id: artist.source_uuid,
        name: artist.name,
    };

    // The artist's songs are those `lib::artist` lists, and its releases those they are on.
    let songs = Query::select()
        .column(media_files::Column::Id)
        .from(media_files::Entity)
        .and_where(artist_uuid_condition(Uuid::parse_str(&artist.id)?))
        .and_where(not_verified_duplicate())
        .to_owned();
    let releases = Query::select()
        .column(discs::Column::AlbumId.as_column_ref())
        .from(audio::Entity)
        .inner_join(
            discs::Entity,
            column(discs::Column::Id).eq(column(audio::Column::DiscId)),
        )
        .and_where(column(audio::Column::FileId).in_subquery(songs.clone()))
        .to_owned();

    // Every song of those releases, and the files of all the songs.
    let mut tracks = Query::select();
    tracks
        .expr_as(column(media_files::Column::Id), "file_id")
        .from(media_files::Entity)
        .and_where(not_verified_duplicate());
    by_release(&mut tracks, media_files::Column::Id, releases.clone());
    let tracks: Vec<TrackRow> = query_models(db, &tracks).await?;
    let release_files = Query::select()
        .column(audio::Column::FileId.as_column_ref())
        .from(audio::Entity)
        .inner_join(
            discs::Entity,
            column(discs::Column::Id).eq(column(audio::Column::DiscId)),
        )
        .and_where(column(discs::Column::AlbumId).in_subquery(releases.clone()))
        .to_owned();
    let files: HashMap<i32, media_files::Model> = media_files::Entity::find()
        .filter(
            media_files::Column::Id
                .in_subquery(songs.clone())
                .or(media_files::Column::Id.in_subquery(release_files)),
        )
        .filter(not_verified_duplicate())
        .all(db)
        .await?
        .into_iter()
        .map(|file| (file.id, file))
        .collect();
    let mut song_ids = db
        .query_all(&songs)
        .await?
        .iter()
        .map(|row| row.try_get::<i32>("", "id"))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    song_ids.retain(|id| files.contains_key(id));

    // What each release is, as the album collection names it and its summary resolves it.
    use album_summaries::Column as S;
    let rows: Vec<ReleaseRow> = query_models(
        db,
        &Query::select()
            .columns(["id", "name", "source_uuid"].map(|c| (Alias::new("albums"), Alias::new(c))))
            .columns([S::ReleaseDate, S::ReleaseType, S::TrackCount].map(|c| c.as_column_ref()))
            .from_subquery(browse::edition_albums(releases.clone()), "albums")
            .left_join(
                album_summaries::Entity,
                column(S::AlbumId).eq(field("albums", "source_uuid")),
            )
            .to_owned(),
    )
    .await?;

    // A release takes each flag from the first of its songs that gives it, as the catalog's
    // release metadata does.
    use values::Column as V;
    let mut flags = Query::select();
    flags
        .columns([V::Qualifier, V::Value].map(|c| c.as_column_ref()))
        .from(values::Entity)
        .and_where(V::Scope.eq(CreditScope::Album as i32))
        .and_where(V::Field.eq(SECONDARY_TYPES))
        .and_where(V::Qualifier.is_in(SECTION_FLAGS.map(ReleaseSecondaryType::as_str)));
    by_release(&mut flags, V::FileId, releases.clone());
    flags.order_by(V::Position.as_column_ref(), Order::Asc);
    let mut flagged: BTreeMap<(String, ReleaseSecondaryType), bool> = BTreeMap::new();
    for row in query_models::<FlagRow>(db, &flags).await? {
        if let Some(kind) = ReleaseSecondaryType::from_label(&row.qualifier) {
            flagged
                .entry((row.album_id, kind))
                .or_insert(row.value == "1");
        }
    }

    // A release credits the album artists of all its songs.
    use credits::Column as C;
    let mut album_artists = Query::select();
    album_artists
        .columns([C::CreditedAs, C::ArtistId].map(|c| c.as_column_ref()))
        .from(credits::Entity)
        .and_where(C::Scope.eq(CreditScope::Album as i32))
        .and_where(C::Role.eq(PRIMARY_ARTIST));
    by_release(&mut album_artists, C::FileId, releases.clone());
    let mut credited: HashMap<String, Vec<ReleaseArtist>> = HashMap::new();
    for row in query_models::<CreditRow>(db, &album_artists).await? {
        let credit = ReleaseArtist {
            credited_as: row.credited_as,
            artist_id: row.artist_id,
        };
        let credits = credited.entry(row.album_id).or_default();
        if !credits.contains(&credit) {
            credits.push(credit);
        }
    }

    let mut durations = release_durations(db, releases).await?;
    let mut songs_of: HashMap<String, Vec<ReleaseTrack>> = HashMap::new();
    for track in tracks {
        if let Some(file) = files.get(&track.file_id) {
            songs_of
                .entry(track.album_id)
                .or_default()
                .push(ReleaseTrack {
                    file_id: file.id,
                    file_uuid: file.source_uuid.clone(),
                    cover_art_uuid: file.cover_art_uuid.clone(),
                });
        }
    }

    let mut releases = Vec::new();
    for row in rows {
        let Some(tracks) = songs_of.remove(&row.source_uuid) else {
            continue;
        };
        let flag = |kind| {
            flagged
                .get(&(row.source_uuid.clone(), kind))
                .copied()
                .unwrap_or(false)
        };
        releases.push(Release {
            album_id: row.id,
            compilation: flag(ReleaseSecondaryType::Compilation),
            live: flag(ReleaseSecondaryType::Live),
            album_artists: credited.remove(&row.source_uuid).unwrap_or_default(),
            duration_ms: durations.remove(&row.source_uuid),
            release_type: row
                .release_type
                .as_deref()
                .and_then(ReleaseType::from_label),
            track_count: row
                .track_count
                .and_then(|count| usize::try_from(count).ok())
                .unwrap_or(tracks.len()),
            name: row.name,
            date: row.release_date,
            tracks,
        });
    }
    releases.sort_by(newest_first);

    Ok(Some(ArtistCatalog {
        artist,
        releases,
        files,
        songs: song_ids,
        song_query: songs,
    }))
}

impl ArtistCatalog {
    /// The releases of one section, which have a name to show, newest first.
    fn section(&self, section: ArtistSection) -> impl Iterator<Item = &Release> {
        self.releases.iter().filter(move |release| {
            !release.name.trim().is_empty() && release.section_for(&self.artist) == section
        })
    }

    fn models(&self, ids: impl Iterator<Item = i32>) -> Vec<media_files::Model> {
        ids.filter_map(|id| self.files.get(&id).cloned()).collect()
    }
}

/// The sections of an artist's page that are not empty, in the order the page lists them, or
/// None when the library has no such artist. Each section carries the songs of up to
/// `cover_releases` of its newest releases that have cover art.
pub async fn artist_page_sections(
    db: &DatabaseConnection,
    artist_id: i32,
    cover_releases: usize,
) -> Result<Option<ArtistPageSections>> {
    let Some(catalog) = load_catalog(db, artist_id).await? else {
        return Ok(None);
    };
    let mut sections = Vec::new();
    for section in ArtistSection::ALL {
        let releases: Vec<&Release> = catalog.section(section).collect();
        if releases.is_empty() {
            continue;
        }
        let mut seen = HashSet::new();
        let cover_files = releases
            .iter()
            .filter_map(|release| {
                release.tracks.iter().find(|track| {
                    track
                        .cover_art_uuid
                        .as_ref()
                        .is_some_and(|uuid| seen.insert(uuid.clone()))
                })
            })
            .take(cover_releases)
            .filter_map(|track| catalog.files.get(&track.file_id).cloned())
            .collect();
        sections.push(ArtistSectionSummary {
            section,
            releases: releases.len(),
            cover_files,
        });
    }
    Ok(Some(ArtistPageSections {
        sections,
        has_songs: !catalog.songs.is_empty(),
    }))
}

/// A page of the songs of one section's releases, release by release, newest release first
/// and each in disc and track order. An unknown artist has none. With `cover_only`, songs
/// without cover art are left out before the page is cut.
pub async fn artist_section_files(
    db: &DatabaseConnection,
    artist_id: i32,
    section: ArtistSection,
    cover_only: bool,
    cursor: usize,
    page_size: usize,
) -> Result<Vec<media_files::Model>> {
    let Some(catalog) = load_catalog(db, artist_id).await? else {
        return Ok(Vec::new());
    };
    Ok(catalog.models(
        catalog
            .section(section)
            .flat_map(|release| release.tracks.iter())
            .filter(|track| !cover_only || track.cover_art_uuid.is_some())
            .map(|track| track.file_id)
            .skip(cursor)
            .take(page_size),
    ))
}

/// A page of the album ids of one section's releases in groups by the year of their release
/// dates (see `year_groups`), each year in the section's order: the newest release first. An
/// unknown artist has none.
pub async fn artist_section_album_groups(
    db: &DatabaseConnection,
    artist_id: i32,
    section: ArtistSection,
    cursor: usize,
    page_size: usize,
) -> Result<Vec<YearGroup<i32>>> {
    let Some(catalog) = load_catalog(db, artist_id).await? else {
        return Ok(Vec::new());
    };
    Ok(year_groups(
        catalog
            .section(section)
            .map(|release| (release.album_id, release.year())),
        cursor,
        page_size,
    ))
}

/// A page of the artist's songs, most listened first:
///
/// 1. by how often they were played through, most first, over all devices;
/// 2. on a tie, liked songs first;
/// 3. then fewer skips first;
/// 4. then, which is all there is to tell songs never played apart, the newest release first
///    and each release in disc and track order.
///
/// With `cover_only`, songs without cover art are left out before the page is cut.
pub async fn artist_track_files(
    db: &DatabaseConnection,
    artist_id: i32,
    cover_only: bool,
    cursor: usize,
    page_size: usize,
) -> Result<Vec<media_files::Model>> {
    let Some(catalog) = load_catalog(db, artist_id).await? else {
        return Ok(Vec::new());
    };
    let uuids = || {
        Query::select()
            .column(media_files::Column::SourceUuid)
            .from(media_files::Entity)
            .and_where(media_files::Column::Id.in_subquery(catalog.song_query.clone()))
            .to_owned()
    };
    let mut played: HashMap<String, i64> = HashMap::new();
    let mut skipped: HashMap<String, i64> = HashMap::new();
    let stats = media_file_stats::Entity::find()
        .filter(media_file_stats::Column::TrackUuid.in_subquery(uuids()))
        .all(db)
        .await?;
    for row in stats {
        *played.entry(row.track_uuid.clone()).or_default() += i64::from(row.played_through);
        *skipped.entry(row.track_uuid).or_default() += i64::from(row.skipped);
    }
    let liked: HashSet<String> = media_file_likes::Entity::find()
        .filter(media_file_likes::Column::TrackUuid.in_subquery(uuids()))
        .filter(media_file_likes::Column::Liked.eq(true))
        .all(db)
        .await?
        .into_iter()
        .map(|row| row.track_uuid)
        .collect();

    // Where a song stands in its release, newest release first.
    let mut place: HashMap<&str, usize> = HashMap::new();
    for release in &catalog.releases {
        for track in &release.tracks {
            let next = place.len();
            if let Entry::Vacant(entry) = place.entry(track.file_uuid.as_str()) {
                entry.insert(next);
            }
        }
    }

    let mut order: Vec<(&str, i32)> = catalog
        .songs
        .iter()
        .map(|id| (catalog.files[id].source_uuid.as_str(), *id))
        .collect();
    order.sort_by(|(a, a_id), (b, b_id)| {
        let count = |map: &HashMap<String, i64>, uuid: &str| map.get(uuid).copied().unwrap_or(0);
        count(&played, b)
            .cmp(&count(&played, a))
            .then_with(|| liked.contains(*b).cmp(&liked.contains(*a)))
            .then_with(|| count(&skipped, a).cmp(&count(&skipped, b)))
            .then_with(|| {
                let place = |uuid: &str| place.get(uuid).copied().unwrap_or(usize::MAX);
                place(a).cmp(&place(b))
            })
            .then_with(|| a_id.cmp(b_id))
    });
    Ok(catalog.models(
        order
            .into_iter()
            .map(|(_, id)| id)
            .filter(|id| !cover_only || catalog.files[id].cover_art_uuid.is_some())
            .skip(cursor)
            .take(page_size),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ann() -> PageArtist {
        PageArtist {
            id: "ann-id".into(),
            name: "Ann".into(),
        }
    }

    fn release(tracks: usize, seconds_each: u64) -> Release {
        Release {
            album_id: 1,
            name: "R".into(),
            date: None,
            release_type: None,
            compilation: false,
            live: false,
            album_artists: Vec::new(),
            track_count: tracks,
            duration_ms: Some(tracks as u64 * seconds_each * 1000),
            tracks: Vec::new(),
        }
    }

    fn credit(credited_as: &str, artist_id: Option<&str>) -> ReleaseArtist {
        ReleaseArtist {
            credited_as: credited_as.into(),
            artist_id: artist_id.map(Into::into),
        }
    }

    #[test]
    fn size_decides_single_ep_or_album() {
        use ArtistSection::*;
        let minutes = |minutes: u64| minutes * 60 * 1000;
        assert_eq!(section_by_size(1, minutes(3)), Singles);
        assert_eq!(section_by_size(3, minutes(10)), Singles);
        // An EP is a single here: the page has one "Singles & EPs" section.
        assert_eq!(section_by_size(4, minutes(10)), Singles);
        assert_eq!(section_by_size(6, minutes(28)), Singles);
        assert_eq!(section_by_size(7, minutes(10)), Albums);
        assert_eq!(section_by_size(12, minutes(50)), Albums);
        // Long enough for an album whatever the count.
        assert_eq!(section_by_size(2, minutes(30)), Albums);
        assert_eq!(section_by_size(5, minutes(30)), Albums);
        assert_eq!(section_by_size(3, minutes(30) - 1), Singles);
    }

    #[test]
    fn the_type_decides_before_the_length() {
        use ArtistSection::*;
        let typed = |tracks, seconds, kind| Release {
            release_type: Some(kind),
            ..release(tracks, seconds)
        };
        // Long releases tagged as an EP or a single.
        assert_eq!(typed(5, 600, ReleaseType::Ep).section_for(&ann()), Singles);
        assert_eq!(
            typed(2, 1200, ReleaseType::Single).section_for(&ann()),
            Singles
        );
        // Short releases tagged as an album or a type without a section of its own.
        for kind in [
            ReleaseType::Album,
            ReleaseType::Broadcast,
            ReleaseType::Other,
        ] {
            assert_eq!(typed(2, 100, kind).section_for(&ann()), Albums, "{kind:?}");
        }
        // Without a type, the length decides.
        assert_eq!(release(5, 600).section_for(&ann()), Albums);
        assert_eq!(release(2, 100).section_for(&ann()), Singles);
    }

    #[test]
    fn rules_apply_in_order() {
        use ArtistSection::*;
        let mut r = release(10, 240);
        r.release_type = Some(ReleaseType::Single);
        assert_eq!(r.section_for(&ann()), Singles);
        r.live = true;
        assert_eq!(r.section_for(&ann()), Live);
        r.album_artists = vec![credit("Bob", None)];
        assert_eq!(r.section_for(&ann()), AppearsOn);
        r.album_artists = vec![credit("ann", None)];
        assert_eq!(r.section_for(&ann()), Live);
        r.compilation = true;
        assert_eq!(r.section_for(&ann()), Compilations);
        r.album_artists = vec![credit("Various Artists", None)];
        assert_eq!(r.section_for(&ann()), Compilations);
        // A release by others that nothing flags is one the artist appears on.
        r.compilation = false;
        assert_eq!(r.section_for(&ann()), AppearsOn);
    }

    #[test]
    fn an_album_artist_names_the_artist_by_identity_or_by_name() {
        let by = |credits: Vec<ReleaseArtist>| {
            let mut r = release(10, 240);
            r.album_artists = credits;
            r.section_for(&ann()) != ArtistSection::AppearsOn
        };
        assert!(by(Vec::new()));
        assert!(by(vec![credit("Ann & Bob", None)]));
        assert!(by(vec![credit("Bob", None), credit("ANN", None)]));
        // A credit identified as the artist, whatever name it is written as.
        assert!(by(vec![credit("A.", Some("ann-id"))]));
        assert!(!by(vec![credit("Bob", None)]));
        assert!(!by(vec![credit("Bob", Some("bob-id"))]));
        assert!(!by(vec![credit("Annabel", None)]));
    }

    #[test]
    fn an_album_artist_names_the_artist_by_its_whole_name() {
        // An identified or synced artist can keep a name the separators would split.
        let band = PageArtist {
            id: "band-id".into(),
            name: "Earth, Wind & Fire".into(),
        };
        let mut r = release(10, 240);
        r.album_artists = vec![credit("Earth, Wind & Fire", None)];
        assert_eq!(r.section_for(&band), ArtistSection::Albums);
        r.album_artists = vec![credit("Earth", None)];
        assert_eq!(r.section_for(&band), ArtistSection::AppearsOn);
    }

    #[test]
    fn newest_release_goes_first_and_undated_last() {
        let mut a = release(1, 1);
        a.album_id = 1;
        a.date = Some("2019-05-01".into());
        let mut b = release(1, 1);
        b.album_id = 2;
        b.date = Some("2021".into());
        let mut c = release(1, 1);
        c.album_id = 3;
        let mut all = [c, a, b];
        all.sort_by(newest_first);
        assert_eq!(
            all.iter().map(|r| r.album_id).collect::<Vec<_>>(),
            [2, 1, 3]
        );
    }

    #[test]
    fn the_releases_of_one_year_are_ordered_by_month_and_day() {
        // Newest first, as the catalog writes its dates.
        let dates = [
            "2023",
            "2022-12-31",
            "2022-12",
            "2022-03-05",
            "2022-03",
            "2022-01-01",
            "2022",
            "2021-12-31",
        ];
        let mut releases: Vec<Release> = dates
            .iter()
            .rev()
            .enumerate()
            .map(|(id, date)| Release {
                album_id: id as i32,
                date: Some((*date).to_owned()),
                ..release(1, 1)
            })
            .collect();
        releases.sort_by(newest_first);
        let sorted: Vec<&str> = releases
            .iter()
            .map(|release| release.date.as_deref().unwrap())
            .collect();
        assert_eq!(sorted, dates);
    }

    #[test]
    fn a_year_is_the_first_four_digits_of_a_release_date() {
        assert_eq!(year_of("2024-03-01"), Some(2024));
        assert_eq!(year_of("2024-03"), Some(2024));
        assert_eq!(year_of("2024"), Some(2024));
        assert_eq!(year_of("24"), None);
        assert_eq!(year_of(""), None);
    }

    #[test]
    fn albums_are_grouped_by_year_newest_first_and_undated_last_in_their_order() {
        let albums = [
            ("a", Some(2019)),
            ("b", None),
            ("c", Some(2021)),
            ("d", Some(2019)),
            ("e", None),
            ("f", Some(2021)),
        ];
        let titled = |groups: Vec<YearGroup<&'static str>>| -> Vec<(String, Vec<&str>)> {
            groups
                .into_iter()
                .map(|group| (group.title, group.items))
                .collect()
        };
        assert_eq!(
            titled(year_groups(albums, 0, 10)),
            [
                ("2021".to_owned(), vec!["c", "f"]),
                ("2019".to_owned(), vec!["a", "d"]),
                ("#".to_owned(), vec!["b", "e"]),
            ]
        );
        // A page cuts a year, which the next page goes on with under the same title.
        assert_eq!(
            titled(year_groups(albums, 0, 3)),
            [
                ("2021".to_owned(), vec!["c", "f"]),
                ("2019".to_owned(), vec!["a"]),
            ]
        );
        assert_eq!(
            titled(year_groups(albums, 3, 3)),
            [
                ("2019".to_owned(), vec!["d"]),
                ("#".to_owned(), vec!["b", "e"]),
            ]
        );
        assert!(year_groups(albums, 6, 3).is_empty());
    }
}
