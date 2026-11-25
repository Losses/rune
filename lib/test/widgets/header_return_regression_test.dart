import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../main.dart' show cafeMode;
import '../../providers/linux_custom_window_controls.dart';
import '../../providers/responsive_providers.dart';
import '../../providers/router_path.dart';
import '../../screens/artist/artist_page.dart';
import '../../screens/search/widgets/collection_search_item.dart';
import '../../utils/query_list.dart';
import '../../utils/router/artist_section_parameter.dart';
import '../../utils/router/history.dart';
import '../../utils/router/navigation.dart';
import '../../utils/router/query_tracks_parameter.dart';
import '../../utils/router/router_aware_flyout_controller.dart';
import '../../widgets/ax_reveal/widgets/reveal_effect_context.dart';
import '../../widgets/navigation_bar/flip_animation_manager.dart';
import '../../widgets/navigation_bar/flip_text.dart';
import '../../widgets/navigation_bar/flip_text_animation.dart';
import '../../widgets/navigation_bar/navigation_bar.dart';
import '../../widgets/navigation_bar/parent_link.dart';
import '../../widgets/navigation_bar/slibing_link.dart';
import '../../widgets/router/router_animation.dart';
import '../../widgets/router/rune_with_navigation_bar_and_playback_controllor.dart';
import '../../widgets/start_screen/utils/internal_collection.dart';

/// The menu controller of the search page's result, as its menus own one, for
/// the tests that open a menu.
RouterAwareFlyoutController? _menu;

Widget _target(Widget child) => _menu == null
    ? child
    : FlyoutTarget(controller: _menu!.controller, child: child);

class _SearchPage extends StatefulWidget {
  const _SearchPage();

  @override
  State<_SearchPage> createState() => _SearchPageState();
}

class _SearchPageState extends State<_SearchPage> {
  final controller = ScrollController();

  @override
  void dispose() {
    controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => SingleChildScrollView(
    controller: controller,
    child: Column(
      children: [
        const SizedBox(height: 1500),
        SizedBox(
          height: 80,
          child: _target(
            CollectionSearchItem(
              index: 0,
              collectionType: CollectionType.artist,
              item: InternalCollection(
                id: 7,
                name: 'Some Artist',
                queries: QueryList([('lib::artist', '7')]),
                collectionType: CollectionType.artist,
                coverArtMap: const {},
                readonly: true,
              ),
            ),
          ),
        ),
        const SizedBox(height: 1500),
      ],
    ),
  );
}

/// Every route the app's navigator built for a path, in order.
final Map<String, List<ModalRoute<void>>> _built = {};

Route<void> _route(RouteSettings settings) {
  late final PageRouteBuilder<void> route;
  route = PageRouteBuilder<void>(
    settings: settings,
    transitionDuration: Duration.zero,
    reverseTransitionDuration: Duration.zero,
    pageBuilder: (context, _, _) => RouterAnimation(switch (settings.name) {
      '/search' => const _SearchPage(),
      _ => Text('${settings.name}'),
    }, route: route),
  );
  (_built[settings.name!] ??= []).add(route);
  return route;
}

/// The app's navigator under the real navigation bar, with the providers the
/// two need stubbed to a desktop-sized layout (no force-layout setting, no
/// saved window size) and the localizations the bar's titles read.
Future<void> _pumpApp(WidgetTester tester, String? initial) async {
  tester.view.physicalSize = const Size(1200, 900);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  cafeMode = false;
  _built.clear();
  $router.update('/library', null);
  $history.reset();
  final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider<ScreenSizeProvider>.value(value: sizes),
        ChangeNotifierProvider<ResponsiveProvider>(
          create: (_) => ResponsiveProvider(sizes, () async => null),
        ),
        ChangeNotifierProvider<RouterPathProvider>.value(value: $router),
        ChangeNotifierProvider<LinuxCustomWindowControlsProvider>.value(
          value: LinuxCustomWindowControlsProvider(),
        ),
      ],
      child: FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        navigatorKey: runeWithNavigationBarAndPlaybackControllorNavigatorKey,
        navigatorObservers: [routerAnimationObserver, FlyoutRouteObserver()],
        onGenerateRoute: _route,
        initialRoute: '/library',
        // The flip animation of the titles draws in an overlay above the bar.
        builder: (context, child) => Overlay(
          initialEntries: [
            OverlayEntry(
              builder: (context) => RevealEffectContext(
                child: FlipAnimationManager(
                  child: Column(
                    children: [
                      SizedBox(
                        height: 200,
                        child: ListenableBuilder(
                          listenable: $router,
                          builder: (context, _) =>
                              NavigationBar(path: $router.path),
                        ),
                      ),
                      Expanded(child: child!),
                    ],
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    ),
  );
  await tester.pump(const Duration(seconds: 2));
  if (initial == null) return;
  $push(initial);
  await tester.pumpAndSettle();
}

ModalRoute<void> _routeOf(WidgetTester tester, Finder page) =>
    ModalRoute.of(tester.element(page))! as ModalRoute<void>;

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

/// The links of EGOIST's page as it makes them: Latest Release opens one
/// album, Albums and Singles & EPs their albums, Appears On its songs.
List<ArtistSectionLink> _links() =>
    artistSectionLinks(lookupAppLocalizations(const Locale('en')), [
      _section(
        ArtistSectionKind.latestRelease,
        ArtistSectionOpens.album,
        album: const Album(id: 9, name: 'Nine'),
      ),
      _section(ArtistSectionKind.albums, ArtistSectionOpens.albums),
      _section(ArtistSectionKind.singles, ArtistSectionOpens.albums),
      _section(ArtistSectionKind.appearsOn, ArtistSectionOpens.songs),
    ], 'EGOIST');

/// The artists list and on it the page of EGOIST, then [link] as its page
/// opens it.
Future<void> _openFromArtist(
  WidgetTester tester,
  ArtistSectionLink link,
) async {
  await _pumpApp(tester, '/artists');
  $push('/artists/detail', arguments: QueryTracksParameter(7, 'EGOIST'));
  await tester.pumpAndSettle();
  $push(link.route, arguments: link.arguments);
  await tester.pumpAndSettle();
}

List<SlibingLink> _row(WidgetTester tester) =>
    tester.widgetList<SlibingLink>(find.byType(SlibingLink)).toList();

List<String> _titles(WidgetTester tester) => [
  for (final link in _row(tester))
    link.route.titleBuilder(tester.element(find.byType(NavigationBar))),
];

List<bool> _selected(WidgetTester tester) => [
  for (final link in _row(tester)) link.isSelected,
];

/// The text of [link] the flip animation starts or ends at.
Finder _flipTextOf(SlibingLink link) =>
    find.descendant(of: find.byWidget(link), matching: find.byType(FlipText));

Offset _topLeftOf(WidgetTester tester, SlibingLink link) => tester.getTopLeft(
  find.descendant(of: _flipTextOf(link), matching: find.byType(Transform)),
);

/// The flight of a title that a change of page starts, once it is drawn.
Future<FlipTextAnimation> _flight(WidgetTester tester) async {
  await tester.pump();
  await tester.pump();
  return tester.widget<FlipTextAnimation>(find.byType(FlipTextAnimation));
}

/// Goes somewhere and returns the flights of titles that start, once they
/// are drawn, then lets the pages and the flights settle.
Future<List<FlipTextAnimation>> _flights(
  WidgetTester tester,
  void Function() go,
) async {
  go();
  await tester.pump();
  await tester.pump();
  final flights = tester
      .widgetList<FlipTextAnimation>(find.byType(FlipTextAnimation))
      .toList();
  await tester.pumpAndSettle();
  return flights;
}

List<String> _names() => [for (final entry in $history.history) entry.name];

ParentLink _title(WidgetTester tester) =>
    tester.widget<ParentLink>(find.byType(ParentLink));

void main() {
  testWidgets(
    'opening a search result and returning by the parent title restores '
    'the search page as it was',
    (tester) async {
      await _pumpApp(tester, '/search');
      final searchState = tester.state<_SearchPageState>(
        find.byType(_SearchPage),
      );
      final searchRoute = _routeOf(tester, find.byType(_SearchPage));
      searchState.controller.jumpTo(1400);
      await tester.pump();

      // The real result item, through the real onPressed.
      final item = tester.state<CollectionSearchItemState>(
        find.byType(CollectionSearchItem),
      );
      item.onPressed(tester.element(find.byType(CollectionSearchItem)));
      await tester.pumpAndSettle();

      expect($router.path, '/artists/detail');
      expect($history.history.map((e) => e.name), [
        '/library',
        '/search',
        '/artists/detail',
      ]);

      // The header over the detail page names search; tap it.
      final link = tester.widget<ParentLink>(find.byType(ParentLink));
      link.onPressed();
      await tester.pumpAndSettle();

      expect($router.path, '/search');
      expect(_built['/search'], hasLength(1));
      expect(
        identical(_routeOf(tester, find.byType(_SearchPage)), searchRoute),
        isTrue,
      );
      expect(
        tester.state<_SearchPageState>(find.byType(_SearchPage)),
        same(searchState),
      );
      expect(searchState.controller.offset, 1400);
    },
  );

  testWidgets(
    'a second activation of the parent title before the header rebuilds '
    'does not replace the page the first one returned to',
    (tester) async {
      await _pumpApp(tester, '/search');
      final searchState = tester.state<_SearchPageState>(
        find.byType(_SearchPage),
      );
      final searchRoute = _routeOf(tester, find.byType(_SearchPage));
      searchState.controller.jumpTo(1400);
      await tester.pump();
      tester
          .state<CollectionSearchItemState>(find.byType(CollectionSearchItem))
          .onPressed(tester.element(find.byType(CollectionSearchItem)));
      await tester.pumpAndSettle();

      // Both activations run the callback the detail page's header built.
      final stale = tester
          .widget<ParentLink>(find.byType(ParentLink))
          .onPressed;
      stale();
      stale();
      await tester.pumpAndSettle();

      expect($history.history.map((e) => e.name), ['/library', '/search']);
      expect($router.path, '/search');
      expect(_built['/search'], hasLength(1));
      expect(
        identical(_routeOf(tester, find.byType(_SearchPage)), searchRoute),
        isTrue,
      );
      expect(
        tester.state<_SearchPageState>(find.byType(_SearchPage)),
        same(searchState),
      );
      expect(searchState.controller.offset, 1400);
    },
  );

  testWidgets(
    'the parent title of a page not opened from its parent opens the parent '
    'in its place',
    (tester) async {
      await _pumpApp(tester, '/artists/detail');
      expect($history.history.map((e) => e.name), [
        '/library',
        '/artists/detail',
      ]);

      tester.widget<ParentLink>(find.byType(ParentLink)).onPressed();
      await tester.pumpAndSettle();

      expect(tester.takeException(), isNull);
      expect($router.path, '/artists');
      expect($history.history.map((e) => e.name), ['/library', '/artists']);
      // The router and the history change before the navigator does, so the
      // page on screen is what shows the detail route was really replaced.
      expect(find.text('/artists'), findsOneWidget);
      expect(find.text('/artists/detail'), findsNothing);
    },
  );

  testWidgets(
    'opening search two pages deep from a search result goes back to the '
    'search page as it was instead of opening another',
    (tester) async {
      await _pumpApp(tester, '/search');
      final searchState = tester.state<_SearchPageState>(
        find.byType(_SearchPage),
      );
      final searchRoute = _routeOf(tester, find.byType(_SearchPage));
      searchState.controller.jumpTo(1400);
      await tester.pump();
      tester
          .state<CollectionSearchItemState>(find.byType(CollectionSearchItem))
          .onPressed(tester.element(find.byType(CollectionSearchItem)));
      await tester.pumpAndSettle();
      $push('/albums/detail', arguments: QueryTracksParameter(9, 'Nine'));
      await tester.pumpAndSettle();
      expect(_names(), [
        '/library',
        '/search',
        '/artists/detail',
        '/albums/detail',
      ]);

      // As the search button and its shortcut open it.
      $push('/search');
      await tester.pumpAndSettle();

      expect(_names(), ['/library', '/search']);
      expect($router.path, '/search');
      expect(find.text('/artists/detail'), findsNothing);
      expect(find.text('/albums/detail'), findsNothing);
      expect(_built['/search'], hasLength(1));
      expect(
        identical(_routeOf(tester, find.byType(_SearchPage)), searchRoute),
        isTrue,
      );
      expect(
        tester.state<_SearchPageState>(find.byType(_SearchPage)),
        same(searchState),
      );
      expect(searchState.controller.offset, 1400);

      // Out of search is back to where search was first opened from.
      expect($pop(), isTrue);
      await tester.pumpAndSettle();
      expect(_names(), ['/library']);
    },
  );

  for (final (label, route) in [
    ('Go to Album', '/albums/detail'),
    ('Go to Artist', '/artists/detail'),
  ]) {
    testWidgets(
      'a page opened from a context menu item over search, as $label does, '
      'names search as its parent and goes back to the search as it was',
      (tester) async {
        final controller = _menu = RouterAwareFlyoutController();
        addTearDown(() {
          _menu = null;
          controller.dispose();
        });
        await _pumpApp(tester, '/search');
        final searchState = tester.state<_SearchPageState>(
          find.byType(_SearchPage),
        );
        final searchRoute = _routeOf(tester, find.byType(_SearchPage));
        searchState.controller.jumpTo(1400);
        await tester.pump();

        // The menu as the track menus open it, with an item that closes the
        // flyout and then pushes, as a MenuFlyoutItem does.
        final host = tester.element(find.byType(CollectionSearchItem));
        controller.showFlyout<void>(
          position: const Offset(100, 100),
          builder: (context) => MenuFlyout(
            items: [
              MenuFlyoutItem(
                text: Text(label),
                onPressed: () =>
                    $push(route, arguments: QueryTracksParameter(9, 'Nine')),
              ),
            ],
          ),
          navigatorKey: Navigator.of(host),
        );
        await tester.pumpAndSettle();
        await tester.tap(find.text(label));
        await tester.pumpAndSettle();

        expect($router.path, route);
        expect(_names(), ['/library', '/search', route]);
        expect($history.previousPage?.name, '/search');
        expect(_title(tester).text, 'Search');

        _title(tester).onPressed();
        await tester.pumpAndSettle();

        expect($router.path, '/search');
        expect(_names(), ['/library', '/search']);
        expect(_built['/search'], hasLength(1));
        expect(
          identical(_routeOf(tester, find.byType(_SearchPage)), searchRoute),
          isTrue,
        );
        expect(
          tester.state<_SearchPageState>(find.byType(_SearchPage)),
          same(searchState),
        );
        expect(searchState.controller.offset, 1400);
      },
    );
  }

  testWidgets(
    'the header of a section page lists the sections, a press on another '
    'replaces the page and back is the artist page',
    (tester) async {
      final links = _links();
      await _openFromArtist(tester, links[1]);

      expect(_title(tester).text, 'EGOIST');
      // Latest Release opens one album and so has no link, as the library's
      // row has only its lists.
      expect(_titles(tester), ['Albums', 'Singles & EPs', 'Appears On']);
      expect(_selected(tester), [true, false, false]);

      // Another section of albums: the page is built again with it, and the
      // links stay, as a library link does.
      final singles = tester.state(find.byWidget(_row(tester)[1]));
      await tester.tap(find.byWidget(_row(tester)[1]));
      await tester.pumpAndSettle();

      expect(_names(), [
        '/library',
        '/artists',
        '/artists/detail',
        '/artists/albums',
      ]);
      expect(_built['/artists/albums'], hasLength(2));
      expect(getArtistSectionParameter()!.title, 'Singles & EPs');
      expect(_title(tester).text, 'EGOIST');
      expect(_selected(tester), [false, true, false]);
      expect(tester.state(find.byWidget(_row(tester)[1])), same(singles));

      // A section of songs.
      await tester.tap(find.byWidget(_row(tester)[2]));
      await tester.pumpAndSettle();
      expect($router.path, '/artists/section');
      expect(getArtistSectionParameter()!.title, 'Appears On');
      expect(_selected(tester), [false, false, true]);

      _title(tester).onPressed();
      await tester.pumpAndSettle();
      expect(_names(), ['/library', '/artists', '/artists/detail']);
      expect(_built['/artists/detail'], hasLength(1));
    },
  );

  testWidgets(
    'a section page is under the artist page as a library page is under the '
    'library, and nothing flies from or to the artist page, which has no '
    'link, when it draws no name',
    (tester) async {
      final links = _links();
      await _pumpApp(tester, '/artists');
      $push('/artists/detail', arguments: QueryTracksParameter(7, 'EGOIST'));
      await tester.pumpAndSettle();

      // The pages here are stand-ins that draw no name. The real artist page
      // draws its name, which flies to this title: see title_flight_test.
      expect(
        await _flights(
          tester,
          () => $push(links[3].route, arguments: links[3].arguments),
        ),
        isEmpty,
      );
      expect($router.path, '/artists/section');
      expect(_title(tester).text, 'EGOIST');
      expect(_title(tester).titleFlipKey, 'title:/artists/detail');
      expect(_selected(tester), [false, false, true]);

      // Between sections nothing flies, as between Albums and Artists.
      expect(
        await _flights(tester, () => _row(tester)[0].onPressed()),
        isEmpty,
      );
      expect($router.path, '/artists/albums');
      expect(_title(tester).text, 'EGOIST');
      expect(_selected(tester), [true, false, false]);

      // The title goes back to the artist page as it was left.
      expect(await _flights(tester, _title(tester).onPressed), isEmpty);
      expect(_names(), ['/library', '/artists', '/artists/detail']);
      expect(_built['/artists/detail'], hasLength(1));
      expect(_title(tester).text, 'Artists');
      expect(_row(tester), isEmpty);
    },
  );

  testWidgets(
    'a section page of an artist found in search names the artist, and the '
    'artist page under it search, and nothing flies where no name is drawn',
    (tester) async {
      final links = _links();
      await _pumpApp(tester, '/search');
      expect(
        await _flights(
          tester,
          () => $push(
            '/artists/detail',
            arguments: QueryTracksParameter(7, 'EGOIST'),
          ),
        ),
        isEmpty,
      );
      expect(_title(tester).text, 'Search');

      expect(
        await _flights(
          tester,
          () => $push(links[1].route, arguments: links[1].arguments),
        ),
        isEmpty,
      );
      expect(_title(tester).text, 'EGOIST');
      expect(_selected(tester), [true, false, false]);

      expect(await _flights(tester, _title(tester).onPressed), isEmpty);
      expect(_names(), ['/library', '/search', '/artists/detail']);
      expect(_title(tester).text, 'Search');
    },
  );

  testWidgets(
    'after a press on another section of albums the title flies from the '
    'link of the section shown',
    (tester) async {
      final links = _links();
      await _openFromArtist(tester, links[1]);
      await tester.tap(find.byWidget(_row(tester)[1]));
      await tester.pumpAndSettle();
      final singles = _row(tester)[1];
      expect(
        tester.widget<FlipText>(_flipTextOf(singles)).flipKey,
        'child:/artists/albums',
      );
      final singlesAt = _topLeftOf(tester, singles);
      final albumsAt = _topLeftOf(tester, _row(tester)[0]);

      $push('/albums/detail', arguments: QueryTracksParameter(3, 'Three'));
      final up = await _flight(tester);
      expect(up.fromPositions.position, singlesAt);
      expect(up.fromPositions.position, isNot(albumsAt));
      expect(up.text, 'Singles & EPs');
      await tester.pumpAndSettle();
      expect(_title(tester).text, 'Singles & EPs');
    },
  );

  testWidgets(
    'an album opened from the artist page names the artist, which goes back '
    'to the artist page, and nothing flies where no name is drawn',
    (tester) async {
      final links = _links();
      await _pumpApp(tester, '/artists');
      $push('/artists/detail', arguments: QueryTracksParameter(7, 'EGOIST'));
      await tester.pumpAndSettle();

      expect(
        await _flights(
          tester,
          () => $push(links[0].route, arguments: links[0].arguments),
        ),
        isEmpty,
      );
      expect($router.path, '/albums/detail');
      expect(_title(tester).text, 'EGOIST');
      expect(_title(tester).titleFlipKey, 'title:/artists/detail');
      expect(_row(tester), isEmpty);

      expect(await _flights(tester, _title(tester).onPressed), isEmpty);
      expect(_names(), ['/library', '/artists', '/artists/detail']);
      expect(_built['/artists/detail'], hasLength(1));
      expect(_title(tester).text, 'Artists');
    },
  );

  testWidgets(
    'a library page takes the link of the library into its title, with the '
    "library's pages in its row",
    (tester) async {
      await _pumpApp(tester, null);
      final link = _row(tester).singleWhere((x) => x.route.path == '/library');
      final at = _topLeftOf(tester, link);

      final up = await _flights(tester, () => $push('/albums'));
      expect(up, hasLength(1));
      expect(up.single.text, 'Library');
      expect(up.single.fromPositions.position, at);
      expect(_title(tester).text, 'Library');
      expect(_titles(tester), [
        'Artists',
        'Albums',
        'Genres',
        'Playlists',
        'Mixes',
        'Tracks',
      ]);
      expect(
        _row(tester).singleWhere((x) => x.isSelected).route.path,
        '/albums',
      );
    },
  );

  testWidgets(
    'the library home opens an album under the albums with no flight, and '
    'the links of the library switch with none',
    (tester) async {
      await _pumpApp(tester, null);
      expect(_title(tester).text, 'Rune');
      expect(
        _row(tester).singleWhere((x) => x.isSelected).route.path,
        '/library',
      );

      expect(
        await _flights(
          tester,
          () => $push(
            '/albums/detail',
            arguments: QueryTracksParameter(3, 'Three'),
          ),
        ),
        isEmpty,
      );
      expect(_names(), ['/library', '/albums/detail']);
      expect(_title(tester).text, 'Albums');
      expect(_row(tester), isEmpty);

      // The albums are not under the album, so the title opens them afresh.
      _title(tester).onPressed();
      await tester.pumpAndSettle();
      expect(_names(), ['/library', '/albums']);
      expect(_title(tester).text, 'Library');

      final artists = _row(
        tester,
      ).firstWhere((x) => x.route.path == '/artists');
      expect(await _flights(tester, artists.onPressed), isEmpty);
      expect(_names(), ['/library', '/artists']);
      expect(_title(tester).text, 'Library');
      expect(
        _row(tester).singleWhere((x) => x.isSelected).route.path,
        '/artists',
      );
    },
  );

  testWidgets(
    'the library keeps its headers: an artist under the artists, an album '
    'under the albums',
    (tester) async {
      await _pumpApp(tester, '/artists');
      $push('/artists/detail', arguments: QueryTracksParameter(7, 'EGOIST'));
      await tester.pumpAndSettle();
      expect(_title(tester).text, 'Artists');
      expect(_title(tester).titleFlipKey, 'title:/artists');

      $push('/albums');
      await tester.pumpAndSettle();
      expect(_title(tester).text, 'Library');
      expect(_titles(tester), contains('Albums'));

      $push('/albums/detail', arguments: QueryTracksParameter(3, 'Three'));
      final up = await _flight(tester);
      expect(up.text, 'Albums');
      await tester.pumpAndSettle();
      expect(_title(tester).text, 'Albums');
      expect(_title(tester).titleFlipKey, 'title:/albums');
    },
  );

  testWidgets(
    'a link to a page of the navigation replaces the page with it, whatever '
    'a press on its title does',
    (tester) async {
      // A page outside the navigation has the root items in its row.
      await _pumpApp(tester, '/lyrics');
      expect(_titles(tester), ['Rune', 'Search']);

      // Without a title above it the row is drawn above the window here.
      final before = _names();
      expect(before.last, '/lyrics');
      _row(tester)[0].onPressed();
      await tester.pumpAndSettle();
      expect($router.path, '/');
      // The root replaced /lyrics: one entry deep as before, and back does
      // not lead to /lyrics.
      expect(_names(), [...before.take(before.length - 1), '/']);
    },
  );
}
