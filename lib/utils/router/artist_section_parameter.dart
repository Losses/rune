import '../query_list.dart';
import 'query_tracks_parameter.dart';
import 'router_transition_parameter.dart';

/// The routes an artist's page and the pages of its sections open.
const artistDetailRoute = '/artists/detail';
const artistSectionTracksRoute = '/artists/section';
const artistSectionAlbumsRoute = '/artists/albums';
const albumDetailRoute = '/albums/detail';

/// A section of an artist's page as its tile opens it: the name it goes by,
/// and the route and the arguments to push.
typedef ArtistSectionLink = ({String title, String route, Object arguments});

/// A section of an artist's page as a link in the header's row names and opens
/// it: the name it goes by, the route of its page and the queries of its
/// songs.
typedef ArtistSectionRowLink = ({
  String title,
  String route,
  QueryList queries,
});

/// What the page of one section of an artist opens with: the queries of its
/// songs, the name it is titled by and the artist it belongs to.
class ArtistSectionParameter {
  final QueryList queries;
  final String title;
  final String artistName;

  /// The sections of the artist's page that open a page of songs or albums,
  /// in its order, which the header lists above this one's page, and the
  /// index of this one among them.
  final List<ArtistSectionRowLink> sections;
  final int index;

  const ArtistSectionParameter(
    this.queries,
    this.title,
    this.artistName, {
    this.sections = const [],
    this.index = 0,
  });

  /// What the page of the section at [index] of the row opens with.
  ArtistSectionParameter at(int index) => ArtistSectionParameter(
    sections[index].queries,
    sections[index].title,
    artistName,
    sections: sections,
    index: index,
  );
}

/// What the album page opens with when an artist's page opens the album as
/// one of its sections. Only then does the header name the artist above the
/// album: one opened from the artist's page any other way, as a track's menu
/// opens one, is under the albums.
class ArtistAlbumParameter extends QueryTracksParameter {
  ArtistAlbumParameter(super.id, super.title);
}

ArtistSectionParameter? getArtistSectionParameter() {
  final parameters = getRouterTransitionParameter()?.parameters;

  if (parameters is! ArtistSectionParameter) {
    return null;
  }

  return parameters;
}
