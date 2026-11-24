//! The operators of the mix query language (see `documents/mix_query_syntax.md`).
//!
//! The syntax is text: an operator such as `lib::artist` with a text parameter. This module is
//! the only place that reads and writes the operator text; everything else works with
//! [`MixOperator`].

/// An operator of the mix query language, without its parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MixOperator {
    LibAll,
    LibArtist,
    LibAlbum,
    LibGenre,
    LibPlaylist,
    LibTrack,
    LibRandom,
    LibQueue,
    LibDirectoryDeep,
    LibDirectoryShallow,
    SortTrackNumber,
    SortLastModified,
    SortDuration,
    SortPlayedthrough,
    SortSkipped,
    FilterLiked,
    FilterWithCoverArt,
    FilterAnalyzed,
    PipeLimit,
    PipeRecommend,
}

/// What kind of text an operator takes as its parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParameterKind {
    /// `true` or `false`.
    Flag,
    /// A signed number.
    Number,
    /// An unsigned number.
    Limit,
    /// The row of a library collection, stored as the identity of that row.
    Reference(MixTarget),
    /// A directory path of this device.
    Directory,
}

/// The collections a mix clause can point at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MixTarget {
    Tracks,
    Albums,
    Artists,
    Genres,
    Playlists,
}

impl MixTarget {
    /// The tag the synced collection is registered under.
    pub fn tag(self) -> &'static str {
        match self {
            Self::Tracks => "tracks",
            Self::Albums => "albums",
            Self::Artists => "artists",
            Self::Genres => "genres",
            Self::Playlists => "playlists",
        }
    }
}

/// The text names no operator of the language.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown mix query operator `{0}`")]
pub struct UnknownOperator(pub String);

impl MixOperator {
    pub const ALL: [MixOperator; 20] = [
        Self::LibAll,
        Self::LibArtist,
        Self::LibAlbum,
        Self::LibGenre,
        Self::LibPlaylist,
        Self::LibTrack,
        Self::LibRandom,
        Self::LibQueue,
        Self::LibDirectoryDeep,
        Self::LibDirectoryShallow,
        Self::SortTrackNumber,
        Self::SortLastModified,
        Self::SortDuration,
        Self::SortPlayedthrough,
        Self::SortSkipped,
        Self::FilterLiked,
        Self::FilterWithCoverArt,
        Self::FilterAnalyzed,
        Self::PipeLimit,
        Self::PipeRecommend,
    ];

    /// The operator as the syntax writes it.
    pub fn syntax(self) -> &'static str {
        match self {
            Self::LibAll => "lib::all",
            Self::LibArtist => "lib::artist",
            Self::LibAlbum => "lib::album",
            Self::LibGenre => "lib::genre",
            Self::LibPlaylist => "lib::playlist",
            Self::LibTrack => "lib::track",
            Self::LibRandom => "lib::random",
            Self::LibQueue => "lib::queue",
            Self::LibDirectoryDeep => "lib::directory.deep",
            Self::LibDirectoryShallow => "lib::directory.shallow",
            Self::SortTrackNumber => "sort::track_number",
            Self::SortLastModified => "sort::last_modified",
            Self::SortDuration => "sort::duration",
            Self::SortPlayedthrough => "sort::playedthrough",
            Self::SortSkipped => "sort::skipped",
            Self::FilterLiked => "filter::liked",
            Self::FilterWithCoverArt => "filter::with_cover_art",
            Self::FilterAnalyzed => "filter::analyzed",
            Self::PipeLimit => "pipe::limit",
            Self::PipeRecommend => "pipe::recommend",
        }
    }

    /// Reads an operator written in the syntax.
    pub fn from_syntax(text: &str) -> Result<Self, UnknownOperator> {
        Self::ALL
            .into_iter()
            .find(|operator| operator.syntax() == text)
            .ok_or_else(|| UnknownOperator(text.to_owned()))
    }

    pub fn parameter_kind(self) -> ParameterKind {
        match self {
            Self::LibArtist => ParameterKind::Reference(MixTarget::Artists),
            Self::LibAlbum => ParameterKind::Reference(MixTarget::Albums),
            Self::LibGenre => ParameterKind::Reference(MixTarget::Genres),
            Self::LibPlaylist => ParameterKind::Reference(MixTarget::Playlists),
            Self::LibTrack => ParameterKind::Reference(MixTarget::Tracks),
            Self::LibDirectoryDeep | Self::LibDirectoryShallow => ParameterKind::Directory,
            Self::LibAll
            | Self::LibQueue
            | Self::SortTrackNumber
            | Self::SortLastModified
            | Self::SortDuration
            | Self::SortPlayedthrough
            | Self::SortSkipped
            | Self::FilterLiked
            | Self::FilterWithCoverArt
            | Self::FilterAnalyzed => ParameterKind::Flag,
            Self::LibRandom | Self::PipeRecommend => ParameterKind::Number,
            Self::PipeLimit => ParameterKind::Limit,
        }
    }

    /// The collection the parameter names a row of, for operators that point at one.
    pub fn target_collection(self) -> Option<MixTarget> {
        match self.parameter_kind() {
            ParameterKind::Reference(collection) => Some(collection),
            _ => None,
        }
    }

    pub fn is_directory(self) -> bool {
        self.parameter_kind() == ParameterKind::Directory
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_operator_round_trips_through_its_syntax() {
        for operator in MixOperator::ALL {
            assert_eq!(MixOperator::from_syntax(operator.syntax()), Ok(operator));
        }
        let names = MixOperator::ALL.map(MixOperator::syntax);
        for (index, name) in names.iter().enumerate() {
            assert!(!names[..index].contains(name), "{name} is written twice");
        }
    }

    #[test]
    fn text_that_is_not_an_operator_is_a_typed_error() {
        for text in ["", "lib::nothing", "LIB::ARTIST", "lib::artist ", "artist"] {
            assert_eq!(
                MixOperator::from_syntax(text),
                Err(UnknownOperator(text.to_owned()))
            );
        }
    }
}
