use anyhow::{Context, Result};
use database::actions::artist_page::{
    SECTION_COVERS, artist_page_sections, artist_section_album_groups,
};
use database::actions::cover_art::bake_cover_art_by_media_files;
use database::actions::mixes::QueryOperator;
use database::error::ExpectedError;
use database::mix_operator::ArtistSection as LibrarySection;

use crate::library_pool::LibraryHandle;
use crate::request_error::ExpectedRequestError;
use crate::utils::{GlobalParams, ParamsExtractor, process_cover_art_map, require_library};
use crate::{Session, Signal, messages::*};

fn mix_query(clause: &QueryOperator) -> MixQuery {
    let (operator, parameter) = clause.syntax();
    MixQuery {
        operator: operator.to_owned(),
        parameter,
    }
}

/// The query that names a section of an artist's page.
pub(crate) fn section_query(artist: i32, section: LibrarySection) -> MixQuery {
    mix_query(&QueryOperator::LibArtistSection(artist, section))
}

/// The query that names an artist's tracks.
pub(crate) fn tracks_query(artist: i32) -> MixQuery {
    mix_query(&QueryOperator::LibArtistTracks(artist))
}

/// The artist and the section a section's queries name.
pub(crate) fn named_section(queries: &[MixQuery]) -> Result<(i32, LibrarySection)> {
    let invalid =
        || ExpectedRequestError::invalid_input("Not the queries of a section of an artist's page");
    let [query] = queries else {
        return Err(invalid().into());
    };
    match QueryOperator::parse(&query.operator, &query.parameter) {
        Ok(QueryOperator::LibArtistSection(artist, section)) => Ok((artist, section)),
        _ => Err(invalid().into()),
    }
}

impl ParamsExtractor for FetchArtistPageRequest {
    type Params = Option<LibraryHandle>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.library.clone()
    }
}

/// What opening a section of the local library's page shows: singles (EPs among them) play
/// as songs, the other sections list their releases as albums.
fn section_opens(section: LibrarySection) -> ArtistSectionOpens {
    match section {
        LibrarySection::Singles | LibrarySection::Eps => ArtistSectionOpens::Songs,
        LibrarySection::Albums
        | LibrarySection::Compilations
        | LibrarySection::Live
        | LibrarySection::AppearsOn
        | LibrarySection::LatestRelease
        | LibrarySection::EssentialAlbums => ArtistSectionOpens::Albums,
    }
}

/// The page of an artist of the open local library: the sections its catalog sorts the
/// artist's releases into, and the artist's tracks.
impl Signal for FetchArtistPageRequest {
    type Params = Option<LibraryHandle>;
    type Response = FetchArtistPageResponse;

    async fn handle(
        &self,
        library: Self::Params,
        session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let artist_id = dart_signal.artist_id;
        let library = require_library(library)?;
        let Some(page) = artist_page_sections(&library.main_db, artist_id, SECTION_COVERS)
            .await
            .with_context(|| "Failed to read the sections of an artist's page")?
        else {
            return Err(ExpectedError::NotFound { what: "artist" }.into());
        };
        let summaries = page.sections;

        let files = summaries
            .iter()
            .flat_map(|summary| summary.cover_files.iter().cloned())
            .collect();
        let paths = process_cover_art_map(
            bake_cover_art_by_media_files(&library.fsio, &library.main_db, files)
                .await
                .with_context(|| "Failed to bake the covers of an artist's page")?,
            library.library_id,
            session.as_ref(),
        );

        let sections = summaries
            .into_iter()
            .map(|summary| ArtistSection {
                kind: summary.section.into(),
                title: String::new(),
                opens: section_opens(summary.section),
                album: None,
                queries: vec![section_query(artist_id, summary.section)],
                covers: summary
                    .cover_files
                    .iter()
                    .filter_map(|file| paths.get(&file.id))
                    .filter(|path| !path.is_empty())
                    .cloned()
                    .collect(),
                count: i32::try_from(summary.releases).unwrap_or(i32::MAX),
            })
            .collect();
        Ok(Some(FetchArtistPageResponse {
            call_id: None,
            artist_id,
            sections,
            tracks: if page.has_songs {
                vec![tracks_query(artist_id)]
            } else {
                Vec::new()
            },
        }))
    }
}

/// The cursor and page size of a request, or an invalid input error for a negative one, as a
/// mix query answers.
pub(crate) fn page_bounds(cursor: i32, page_size: i32) -> Result<(usize, usize)> {
    let cursor = usize::try_from(cursor)
        .map_err(|error| ExpectedRequestError::invalid_input(format!("Invalid cursor: {error}")))?;
    let page_size = usize::try_from(page_size).map_err(|error| {
        ExpectedRequestError::invalid_input(format!("Invalid page size: {error}"))
    })?;
    Ok((cursor, page_size))
}

impl ParamsExtractor for FetchArtistSectionAlbumsRequest {
    type Params = Option<LibraryHandle>;

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        all_params.library.clone()
    }
}

/// A page of the albums of a section of an artist's page of the open local library: its
/// releases' album ids in groups by the year of their release dates, newest release first.
impl Signal for FetchArtistSectionAlbumsRequest {
    type Params = Option<LibraryHandle>;
    type Response = FetchArtistSectionAlbumsResponse;

    async fn handle(
        &self,
        library: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let (artist, section) = named_section(&dart_signal.queries)?;
        let (cursor, page_size) = page_bounds(dart_signal.cursor, dart_signal.page_size)?;
        let groups =
            artist_section_album_groups(&library.main_db, artist, section, cursor, page_size)
                .await
                .with_context(|| "Failed to read the albums of a section of an artist's page")?;
        Ok(Some(FetchArtistSectionAlbumsResponse {
            call_id: None,
            groups: groups
                .into_iter()
                .map(|group| ArtistSectionAlbumGroup {
                    group_title: group.title,
                    album_ids: group.items,
                })
                .collect(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_negative_cursor_or_page_size_is_invalid_input() {
        assert_eq!(page_bounds(0, 20).unwrap(), (0, 20));
        for (cursor, page_size) in [(-1, 20), (0, -1), (-5, -5)] {
            let error = page_bounds(cursor, page_size).unwrap_err();
            assert!(
                matches!(
                    error.downcast_ref::<ExpectedRequestError>(),
                    Some(ExpectedRequestError::InvalidInput { .. })
                ),
                "{cursor} {page_size}"
            );
        }
    }
}
