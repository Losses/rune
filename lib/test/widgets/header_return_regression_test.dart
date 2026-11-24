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
import '../../widgets/ax_reveal/widgets/reveal_effect_context.dart';
import '../../widgets/navigation_bar/flip_animation_manager.dart';
import '../../widgets/navigation_bar/navigation_bar.dart';
import '../../widgets/navigation_bar/parent_link.dart';
import '../../widgets/router/router_animation.dart';
import '../../widgets/router/rune_with_navigation_bar_and_playback_controllor.dart';
import '../../widgets/start_screen/utils/internal_collection.dart';

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
          child: CollectionSearchItem(
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
        navigatorObservers: [routerAnimationObserver],
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
}
