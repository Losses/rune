import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../main.dart' show cafeMode;
import '../../providers/linux_custom_window_controls.dart';
import '../../providers/responsive_providers.dart';
import '../../providers/router_path.dart';
import '../../screens/search/widgets/collection_search_item.dart';
import '../../utils/query_list.dart';
import '../../utils/router/history.dart';
import '../../utils/router/navigation.dart';
import '../../utils/router/query_tracks_parameter.dart';
import '../../utils/router/router_aware_flyout_controller.dart';
import '../../widgets/ax_reveal/widgets/reveal_effect_context.dart';
import '../../widgets/navigation_bar/flip_animation_manager.dart';
import '../../widgets/navigation_bar/navigation_bar.dart';
import '../../widgets/navigation_bar/parent_link.dart';
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
Future<void> _pumpApp(WidgetTester tester, String initial) async {
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
  $push(initial);
  await tester.pumpAndSettle();
}

ModalRoute<void> _routeOf(WidgetTester tester, Finder page) =>
    ModalRoute.of(tester.element(page))! as ModalRoute<void>;

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
}
