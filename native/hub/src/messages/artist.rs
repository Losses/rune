use database::mix_operator::ArtistSection as LibrarySection;
use requests::NamedLibraries;
use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

use super::album::Album;
use super::mix::MixQuery;

#[derive(Clone, Serialize, Deserialize, SignalPiece, Debug)]
pub struct Artist {
    pub id: i32,
    pub name: String,
}

/// Asks for the page of an artist: the artist's releases in sections, and the artist's tracks.
/// A library that does not answer it keeps the artist page it has.
#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct FetchArtistPageRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
    /// The artist, by the id the library gave it.
    pub artist_id: i32,
    /// The language the client shows, as a BCP 47 tag such as `zh-Hans` or `en`. A library
    /// names the page's sections in it where it can, and gives no names where it cannot.
    #[serde(default)]
    pub language: String,
}

/// The page of an artist. Everything on it opens by the queries it carries, which the paged
/// mix query answers.
#[derive(Deserialize, Serialize, RustSignal)]
pub struct FetchArtistPageResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
    pub artist_id: i32,
    /// The release sections, in the order they are listed in, without the empty ones.
    pub sections: Vec<ArtistSection>,
    /// The queries that give the artist's tracks, to be paged through as the list scrolls.
    pub tracks: Vec<MixQuery>,
}

/// One section of an artist's page: a group of the artist's releases. The library decides
/// which sections a page has, their order, their names and what each opens; the client draws
/// them as they come.
#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct ArtistSection {
    /// What the section holds. The client names the section by it only when `title` is empty.
    pub kind: ArtistSectionKind,
    /// The name the library gives the section, in the user's language where it can. Empty when
    /// it gives none.
    #[serde(default)]
    pub title: String,
    /// What opening the section shows.
    #[serde(default)]
    pub opens: ArtistSectionOpens,
    /// The queries that give the songs of the section's releases, release by release. They
    /// also name the section to `FetchArtistSectionAlbumsRequest`.
    pub queries: Vec<MixQuery>,
    /// The album the section opens, when it opens as `ArtistSectionOpens::Album`.
    #[serde(default)]
    pub album: Option<Album>,
    /// The paths of a few of the section's covers that are cached already, for its tile.
    /// More come in later, announced by `LibraryContentChanged`.
    pub covers: Vec<String>,
    /// How many releases the section holds, as far as is known.
    pub count: i32,
}

/// What kind of releases a section of an artist's page holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, SignalPiece)]
pub enum ArtistSectionKind {
    /// The artist's full-length albums.
    Albums,
    /// Releases shorter than an album and longer than a single.
    Eps,
    /// Singles.
    Singles,
    /// Compilations the artist is on.
    Compilations,
    /// Live albums.
    Live,
    /// Releases by other artists that the artist appears on.
    AppearsOn,
    /// The artist's newest release.
    LatestRelease,
    /// The albums the service picks as the artist's essential ones.
    EssentialAlbums,
}

impl From<LibrarySection> for ArtistSectionKind {
    fn from(section: LibrarySection) -> Self {
        match section {
            LibrarySection::Albums => Self::Albums,
            LibrarySection::Eps => Self::Eps,
            LibrarySection::Singles => Self::Singles,
            LibrarySection::Compilations => Self::Compilations,
            LibrarySection::Live => Self::Live,
            LibrarySection::AppearsOn => Self::AppearsOn,
            LibrarySection::LatestRelease => Self::LatestRelease,
            LibrarySection::EssentialAlbums => Self::EssentialAlbums,
        }
    }
}

/// What opening a section of an artist's page shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, SignalPiece)]
pub enum ArtistSectionOpens {
    /// The songs of the section, release after release, as its queries give them.
    #[default]
    Songs,
    /// The section's releases as albums, which `FetchArtistSectionAlbumsRequest` lists.
    Albums,
    /// One album, the section's `album`, opened directly.
    Album,
}

/// Asks for a page of the albums of a section of an artist's page that opens as albums.
#[derive(Serialize, Deserialize, DartSignal, NamedLibraries)]
pub struct FetchArtistSectionAlbumsRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
    /// The section's queries, as the artist page gave them.
    pub queries: Vec<MixQuery>,
    /// How many albums of the section come before the page.
    pub cursor: i32,
    pub page_size: i32,
}

/// A page of the albums of a section of an artist's page, in groups by the year they came out,
/// or, for a section the library curates, as Essential Albums, in one group in its own order.
#[derive(Deserialize, Serialize, RustSignal)]
pub struct FetchArtistSectionAlbumsResponse {
    #[serde(default)]
    pub call_id: Option<u64>,
    /// The page's albums in groups: the newest year first and the albums with no year last,
    /// each group in the section's order. A group the page before ended with goes on at the
    /// start of this one under the same title. A page of fewer albums than asked for is the
    /// last.
    pub groups: Vec<ArtistSectionAlbumGroup>,
}

/// The albums of a section of an artist's page that came out in one year, or all of a curated
/// section's.
#[derive(Clone, Serialize, Deserialize, SignalPiece)]
pub struct ArtistSectionAlbumGroup {
    /// The year as digits, or "#" for the albums whose year is unknown, as the library's
    /// grouped pages title what has no group of its own. Empty for the one group of a curated
    /// section, which the client titles as it titles the section.
    pub group_title: String,
    /// The albums, by the ids the library gives its albums.
    pub album_ids: Vec<i32>,
}
