import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../bindings/bindings.dart';
import '../../config/navigation_query.dart';
import '../../l10n/app_localizations.dart';
import '../../screens/artist/artist_page.dart';
import '../../utils/navigation/header_parent.dart';
import '../../utils/query_list.dart';
import '../../utils/router/artist_section_parameter.dart';
import '../../utils/router/modal_route_entry.dart';
import '../../utils/router/navigation_history.dart';
import '../../utils/router/query_tracks_parameter.dart';
import '../../utils/router/router_transition_parameter.dart';

ArtistSection _section(
  ArtistSectionKind kind,
  ArtistSectionOpens opens, {
  Album? album,
}) => ArtistSection(
  kind: kind,
  title: '',
  opens: opens,
  queries: [MixQuery(operator: 'lib::artist_section', parameter: kind.name)],
  album: album,
  covers: const [],
  count: 1,
);

/// The links of an artist's page as it makes them: Latest Release opens one
/// album, Albums and Singles & EPs their albums, Appears On its songs.
Future<List<ArtistSectionLink>> _links() async => artistSectionLinks(
  await AppLocalizations.delegate.load(const Locale('en')),
  [
    _section(
      ArtistSectionKind.latestRelease,
      ArtistSectionOpens.album,
      album: const Album(id: 9, name: 'Nine'),
    ),
    _section(ArtistSectionKind.albums, ArtistSectionOpens.albums),
    _section(ArtistSectionKind.singles, ArtistSectionOpens.albums),
    _section(ArtistSectionKind.appearsOn, ArtistSectionOpens.songs),
  ],
  'EGOIST',
);

/// The route settings the router pushes [name] with, from [from].
RouteSettings _page(String name, [Object? arguments, String from = '']) =>
    RouteSettings(
      name: name,
      arguments: RouterTransitionParameter(from, name, arguments),
    );

/// The artists list, the artist page of EGOIST and the page of its section
/// [index], as the artist page opens it.
NavigationHistory _onSection(List<ArtistSectionLink> links, int index) =>
    NavigationHistory()
      ..push(_page('/artists'))
      ..push(_page('/artists/detail', QueryTracksParameter(7, 'EGOIST')))
      ..push(_page(links[index].route, links[index].arguments));

void main() {
  test('a page opened from search names search above it', () {
    final history = NavigationHistory()
      ..push(const RouteSettings(name: '/search'))
      ..push(const RouteSettings(name: '/artists/detail'));

    expect(headerParent('/artists/detail', history, false)?.path, '/search');
  });

  test('a page opened from its list names that list above it', () {
    final history = NavigationHistory()
      ..push(const RouteSettings(name: '/artists'))
      ..push(const RouteSettings(name: '/artists/detail'));

    expect(headerParent('/artists/detail', history, false)?.path, '/artists');
  });

  test('a dialog over a page opened from search keeps search above it', () {
    final history = NavigationHistory()
      ..push(const RouteSettings(name: '/search'))
      ..push(const RouteSettings(name: '/artists/detail'))
      ..pushModal(ModalRouteEntry(name: 'modal', pop: () {}));

    expect(headerParent('/artists/detail', history, false)?.path, '/search');
  });

  test('a tap on the page already shown does nothing', () {
    final history = NavigationHistory()
      ..push(const RouteSettings(name: '/search'))
      ..push(const RouteSettings(name: '/artists/detail'));

    expect(
      headerTapAction('/search', '/artists/detail', history),
      HeaderTapAction.pop,
    );
    history.pop();
    expect(
      headerTapAction('/search', '/search', history),
      HeaderTapAction.none,
    );
  });

  test('a tap on a parent that is not below the page replaces', () {
    final history =
        NavigationHistory()..push(const RouteSettings(name: '/artists/detail'));

    expect(
      headerTapAction('/artists', '/artists/detail', history),
      HeaderTapAction.replace,
    );
  });

  group('above a page opened from an artist', () {
    test('a section page names the artist and goes back to it', () async {
      final history = _onSection(await _links(), 1);

      final parent = headerParent('/artists/albums', history, false)!;
      expect(parent.path, '/artists/detail');
      expect(parent.titleBuilder(_NoContext()), 'EGOIST');
      expect(
        headerTapAction(parent.path, '/artists/albums', history),
        HeaderTapAction.pop,
      );
    });

    test('a section page of an artist found in search names the artist, and '
        'the artist page search', () async {
      final links = await _links();
      final history = NavigationHistory()
        ..push(_page('/search'))
        ..push(_page('/artists/detail', QueryTracksParameter(7, 'EGOIST')));
      expect(headerParent('/artists/detail', history, false)?.path, '/search');

      history.push(_page(links[3].route, links[3].arguments));
      final parent = headerParent('/artists/section', history, false)!;
      expect(parent.path, '/artists/detail');
      expect(parent.titleBuilder(_NoContext()), 'EGOIST');
    });

    test('an album opened from the artist page names the artist and goes '
        'back to it', () async {
      final links = await _links();
      final history = NavigationHistory()
        ..push(_page('/artists'))
        ..push(_page('/artists/detail', QueryTracksParameter(7, 'EGOIST')))
        ..push(_page(links[0].route, links[0].arguments));

      final parent = headerParent('/albums/detail', history, false)!;
      expect(parent.path, '/artists/detail');
      expect(parent.titleBuilder(_NoContext()), 'EGOIST');
      expect(
        headerTapAction(parent.path, '/albums/detail', history),
        HeaderTapAction.pop,
      );
    });

    test("an album opened from the artist page as a track's menu opens one "
        'names the albums', () {
      final history = NavigationHistory()
        ..push(_page('/artists'))
        ..push(_page('/artists/detail', QueryTracksParameter(7, 'EGOIST')))
        ..push(_page('/albums/detail', QueryTracksParameter(3, 'Three')));

      expect(headerParent('/albums/detail', history, false)?.path, '/albums');
      expect(headerOpener('/albums/detail', history, false), isNull);
    });

    test('an album opened from the library home names the albums', () {
      final history = NavigationHistory()
        ..push(_page('/albums/detail', QueryTracksParameter(3, 'Three')));

      final parent = headerParent('/albums/detail', history, false)!;
      expect(parent, same(navigationQuery.getItem('/albums', false)));
      expect(
        headerTapAction(parent.path, '/albums/detail', history),
        HeaderTapAction.replace,
      );
    });

    test('only an album opened from the albums of a section names the '
        'section', () async {
      final links = await _links();
      // An album opened from the songs of a section, as a track's menu does.
      final fromSongs = _onSection(links, 3)
        ..push(_page('/albums/detail', QueryTracksParameter(3, 'Three')));
      expect(headerParent('/albums/detail', fromSongs, false)?.path, '/albums');

      // Another artist opened from the albums of a section.
      final artist = _onSection(links, 1)
        ..push(_page('/artists/detail', QueryTracksParameter(8, 'Other')));
      expect(headerParent('/artists/detail', artist, false)?.path, '/artists');
    });

    test('an album opened from a section names the section', () async {
      final history = _onSection(await _links(), 2)
        ..push(_page('/albums/detail', QueryTracksParameter(3, 'Three')));

      final parent = headerParent('/albums/detail', history, false)!;
      expect(parent.path, '/artists/albums');
      expect(parent.titleBuilder(_NoContext()), 'Singles & EPs');
      expect(
        headerTapAction(parent.path, '/albums/detail', history),
        HeaderTapAction.pop,
      );
    });

    test('a dialog over the album keeps the section above it', () async {
      final history = _onSection(await _links(), 1)
        ..push(_page('/albums/detail', QueryTracksParameter(3, 'Three')))
        ..pushModal(ModalRouteEntry(name: 'modal', pop: () {}));

      expect(
        headerParent('/albums/detail', history, false)?.path,
        '/artists/albums',
      );
    });

    test('a page opened from search still names search', () async {
      final history = NavigationHistory()
        ..push(_page('/artists/detail', QueryTracksParameter(7, 'EGOIST')))
        ..push(_page('/search'))
        ..push(_page('/albums/detail', QueryTracksParameter(3, 'Three')));

      expect(headerParent('/albums/detail', history, false)?.path, '/search');
    });

    test('a page of the library keeps its parent', () {
      NavigationHistory fromArtist(String name) => NavigationHistory()
        ..push(_page('/artists/detail', QueryTracksParameter(7, 'EGOIST')))
        ..push(_page(name));

      expect(
        headerParent('/albums', fromArtist('/albums'), false)?.path,
        '/library',
      );
      expect(headerParent('/search', fromArtist('/search'), false), isNull);
      expect(
        headerParent('/search', fromArtist('/search'), true)?.path,
        '/library',
      );
    });

    test('another artist opened from an artist names the artists', () {
      final history = NavigationHistory()
        ..push(_page('/artists/detail', QueryTracksParameter(7, 'EGOIST')))
        ..push(_page('/artists/detail', QueryTracksParameter(8, 'Other')));

      expect(headerParent('/artists/detail', history, false)?.path, '/artists');
    });

    test('an album opened from the albums still names the albums', () {
      final history = NavigationHistory()
        ..push(_page('/albums'))
        ..push(_page('/albums/detail', QueryTracksParameter(3, 'Three')));

      expect(headerParent('/albums/detail', history, false)?.path, '/albums');
    });
  });

  group('the row', () {
    test('above a section page lists the sections that open a list in '
        'order, the page selected', () async {
      final links = await _links();
      final history = _onSection(links, 2);

      final row = headerRow('/artists/albums', history, false);
      final titles = row.links!.map((x) => x.titleBuilder(_NoContext()));
      expect(titles, ['Albums', 'Singles & EPs', 'Appears On']);
      expect(row.links!.indexOf(row.current!), 1);
      expect(row.current!.titleBuilder(_NoContext()), 'Singles & EPs');
      expect(row.links!.where((x) => x.hidden), isEmpty);

      // The selected link flips with the title above what the page opens.
      final current = row.current!;
      expect(current.path, '/artists/albums');
      expect(current.onTap, isNull);
      history.push(_page('/albums/detail', QueryTracksParameter(3, 'Three')));
      expect(
        headerParent('/albums/detail', history, false)?.path,
        current.path,
      );

      // The others flip with nothing and keep their links apart.
      final others = row.links!.where((x) => x != current);
      expect(others.map((x) => x.path), isNot(contains('/artists/albums')));
      expect(others.map((x) => x.path).toSet(), hasLength(2));
      expect(others.every((x) => x.onTap != null), isTrue);
      expect(row.links!.map((x) => x.linkKey).toSet(), hasLength(3));
    });

    test("keeps each section's link key from one section to another", () async {
      final links = await _links();
      List<String?> keys(int index) => headerRow(
        links[index].route,
        _onSection(links, index),
        false,
      ).links!.map((x) => x.linkKey).toList();

      expect(keys(1), keys(2));
      expect(keys(1), keys(3));
    });

    test('above any other page is its siblings, the page selected', () {
      final history = NavigationHistory()..push(_page('/albums'));

      final row = headerRow('/albums', history, false);
      expect(row.links, navigationQuery.getSiblings('/albums', false));
      expect(row.current, same(navigationQuery.getItem('/albums', false)));
      expect(row.links!.every((x) => x.linkKey == null), isTrue);
    });

    test('above a section page with no list is its siblings', () {
      final history = NavigationHistory()
        ..push(_page('/artists/detail', QueryTracksParameter(7, 'EGOIST')))
        ..push(
          _page(
            '/artists/albums',
            const ArtistSectionParameter(QueryList(), 'Albums', 'EGOIST'),
          ),
        );

      final row = headerRow('/artists/albums', history, false);
      expect(row.links!.where((x) => !x.hidden), isEmpty);
    });
  });
}

/// The titles read here take nothing from the context.
class _NoContext extends Fake implements BuildContext {}
