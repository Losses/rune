import '../../config/navigation_query.dart';
import '../router/artist_section_parameter.dart';
import '../router/navigation.dart';
import '../router/navigation_history.dart';
import '../router/query_tracks_parameter.dart';
import '../router/route_entry.dart';
import '../router/router_transition_parameter.dart';
import 'navigation_item.dart';

/// The page the title above [path] names and goes to: search for a page opened
/// from it, so that going back keeps what was found, and else the page's own
/// parent. An artist's pages follow the library's:
///
/// - the page of a section is under the artist's page as a library page is
///   under the library, and the title names the artist;
/// - an album opened from the albums of a section is under that page as an
///   album is under the albums, and the title names the section;
/// - an album the artist's page opens as one of its sections, as a section of
///   one album opens, is under that page, and the title names the artist.
NavigationItem? headerParent(
  String? path,
  NavigationHistory history,
  bool isZune,
) => _named(path, history, isZune).parent;

/// The page of history the title above [path] names, when it names one: the
/// artist's page under the page of one of its sections, and the page an album
/// was opened from when the title names that page. The title flips only to
/// and from this page, not another one with its path. A page of the library's
/// own, which has a link in a row, is named by its path alone and has none.
RouteEntry? headerOpener(
  String? path,
  NavigationHistory history,
  bool isZune,
) => _named(path, history, isZune).opener;

({NavigationItem? parent, RouteEntry? opener}) _named(
  String? path,
  NavigationHistory history,
  bool isZune,
) {
  const search = '/search';
  if (path != search && history.previousPageIs(search)) {
    return (parent: navigationQuery.getItem(search, isZune), opener: null);
  }
  final parent = navigationQuery.getParent(path, isZune);
  final shown = _shown(path, history);
  final previous = history.previousPage;
  if (shown is ArtistSectionParameter && parent != null) {
    return (
      parent: NavigationItem(
        (_) => shown.artistName,
        parent.path,
        hidden: true,
      ),
      opener: previous?.name == parent.path ? previous : null,
    );
  }
  if (path == albumDetailRoute && previous != null) {
    final title = switch ((previous.name, _parameters(previous.arguments))) {
      (artistSectionAlbumsRoute, ArtistSectionParameter(:final title)) => title,
      (artistDetailRoute, QueryTracksParameter(:final title))
          when shown is ArtistAlbumParameter =>
        title,
      _ => null,
    };
    if (title != null) {
      return (
        parent: NavigationItem((_) => title, previous.name, hidden: true),
        opener: previous,
      );
    }
  }
  return (parent: parent, opener: null);
}

/// What the page shown was pushed with, when it is the page at [path].
Object? _shown(String? path, NavigationHistory history) {
  final page = history.currentPage;
  return page?.name == path ? _parameters(page?.arguments) : null;
}

/// The links of the header's row above [path], and the one of them that is
/// the page. Above the page of a section of an artist they are all the
/// artist's sections, as the library's pages are above one of them, and a
/// press on another replaces the page with that section's. Else they are the
/// page's siblings.
({List<NavigationItem>? links, NavigationItem? current}) headerRow(
  String? path,
  NavigationHistory history,
  bool isZune,
) {
  final shown = _shown(path, history);
  if (path != null &&
      shown is ArtistSectionParameter &&
      shown.index < shown.sections.length) {
    final links = [
      for (final (index, section) in shown.sections.indexed)
        index == shown.index
            ? NavigationItem(
                (_) => section.title,
                path,
                linkKey: 'section:$index',
              )
            : NavigationItem(
                (_) => section.title,
                // Not the page's path, so that only the link of the section
                // shown flips with the title.
                '${section.route}#$index',
                linkKey: 'section:$index',
                onTap: (_) =>
                    $replace(section.route, arguments: shown.at(index)),
              ),
    ];
    return (links: links, current: links[shown.index]);
  }
  return (
    links: navigationQuery.getSiblings(path, isZune),
    current: navigationQuery.getItem(path, isZune),
  );
}

/// What a page was pushed with, out of the transition the router wraps it in.
Object? _parameters(Object? arguments) =>
    arguments is RouterTransitionParameter ? arguments.parameters : arguments;

enum HeaderTapAction { none, pop, replace }

/// What a tap on the parent title [target] does above the page [currentPath].
/// A tap on the page already shown does nothing, so that a second tap before
/// the header rebuilds cannot replace the page it came from.
HeaderTapAction headerTapAction(
  String target,
  String? currentPath,
  NavigationHistory history,
) {
  if (target == currentPath) return HeaderTapAction.none;
  if (history.previousIs(target)) return HeaderTapAction.pop;
  return HeaderTapAction.replace;
}
