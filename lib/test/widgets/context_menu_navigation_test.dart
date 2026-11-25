import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart' show kSecondaryButton;
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../main.dart' show cafeMode;
import '../../providers/library_open_state.dart';
import '../../providers/library_registry.dart';
import '../../providers/linux_custom_window_controls.dart';
import '../../providers/responsive_providers.dart';
import '../../providers/router_path.dart';
import '../../utils/context_menu/track_item_context_menu.dart';
import '../../utils/navigation/utils/navigation_backward.dart';
import '../../utils/router/history.dart';
import '../../utils/router/modal_route_wrapper.dart';
import '../../utils/router/navigation.dart';
import '../../utils/router/query_tracks_parameter.dart';
import '../../utils/router/router_aware_flyout_controller.dart';
import '../../widgets/ax_reveal/widgets/reveal_effect_context.dart';
import '../../widgets/context_menu_wrapper.dart';
import '../../widgets/navigation_bar/flip_animation_manager.dart';
import '../../widgets/navigation_bar/navigation_bar.dart';
import '../../widgets/navigation_bar/parent_link.dart';
import '../../widgets/router/no_effect_page_route.dart';
import '../../widgets/router/router_animation.dart';
import '../../widgets/router/rune_with_navigation_bar_and_playback_controllor.dart';

const _album = '/albums/detail';
const _artist = '/artists/detail';
const _search = '/search';

/// How a menu item that opens a page closes its menu.
enum _Close {
  /// As a [MenuFlyoutItem] does: it asks the navigator to pop, which happens
  /// only after the item ran.
  item('closed after their items ran'),

  /// As an item that closes its flyout itself does: the flyout's route is
  /// already popping when the page is pushed.
  flyout('closed before their items push');

  const _Close(this.how);

  final String how;
}

var _close = _Close.item;

/// How the menus' items open their page: pushed on the current page, or as the
/// app's Go to items do.
var _navigate = $push;

/// The data the menus wait for before they open, as the track menus load
/// theirs. Null when it is there at once.
Completer<void>? _loading;

/// Whether the songs open the app's own track menu, with its own Go to items,
/// instead of the menu above.
var _trackMenu = false;

/// The song the app's track menu is opened for, on the album and by the
/// artist the menu above goes to.
const _song = FetchParsedMediaFileResponse(
  file: MediaFile(
    id: 12,
    libraryId: 'library-1',
    track: 'track-12',
    path: 'Nine/Song.flac',
    artist: 'EGOIST',
    album: 'Nine',
    title: 'Song',
    duration: 180,
    audioAvailability: AudioAvailability.local,
    audioOrigin: AudioOrigin.host,
  ),
  artists: [Artist(id: 7, name: 'EGOIST')],
  album: Album(id: 9, name: 'Nine'),
);

/// A song's row as the track lists build it: it owns its menu controller,
/// which it lets go of when it goes, and opens the song's menu on a secondary
/// tap with Go to Album and Go to Artist.
class _Row extends StatefulWidget {
  const _Row(this.page);

  final String page;

  @override
  State<_Row> createState() => _RowState();
}

class _RowState extends State<_Row> {
  final _controller = RouterAwareFlyoutController();
  final _attachKey = GlobalKey();

  @override
  void dispose() {
    super.dispose();
    _controller.dispose();
  }

  Future<void> _open(Offset position) async {
    if (_trackMenu) {
      openTrackItemContextMenu(
        position,
        context,
        _attachKey,
        _controller,
        null,
        _song.file.id,
        null,
        null,
        loadPlaylists: () async => const [],
        loadMixes: () async => const [],
        loadMediaFile: (_) async => _song,
      );
      return;
    }
    await _loading?.future;
    if (!mounted) return;
    _controller.showFlyout<void>(
      position: const Offset(100, 300),
      builder: (context) => MenuFlyout(
        items: [
          for (final (label, route, arguments) in [
            ('Go to Album', _album, QueryTracksParameter(9, 'Nine')),
            ('Go to Artist', _artist, QueryTracksParameter(7, 'EGOIST')),
          ])
            MenuFlyoutItem(
              text: Text(label),
              closeAfterClick: _close == _Close.item,
              onPressed: () {
                if (_close == _Close.flyout) Flyout.of(context).close();
                _navigate(route, arguments: arguments);
              },
            ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) => ContextMenuWrapper(
    contextAttachKey: _attachKey,
    contextController: _controller,
    onMiddleClick: (_) {},
    onContextMenu: _open,
    child: SizedBox(
      key: ValueKey('row ${widget.page}'),
      width: 400,
      height: 40,
      child: Text('a song on ${widget.page}'),
    ),
  );
}

/// A page with a song's row, whose state tells one page from another.
class _Page extends StatefulWidget {
  const _Page(this.name);

  final String name;

  @override
  State<_Page> createState() => _PageState();
}

class _PageState extends State<_Page> {
  /// Whether the song is still listed, as a list read again may drop it.
  bool listed = true;

  @override
  Widget build(BuildContext context) => Column(
    children: [Text('page ${widget.name}'), if (listed) _Row(widget.name)],
  );
}

/// The routes the app's navigator holds, in order, as its observers hear of
/// them.
class _Routes extends NavigatorObserver {
  final List<Route<dynamic>> routes = [];

  @override
  void didPush(Route<dynamic> route, Route<dynamic>? previousRoute) =>
      routes.add(route);

  @override
  void didPop(Route<dynamic> route, Route<dynamic>? previousRoute) =>
      routes.remove(route);

  @override
  void didRemove(Route<dynamic> route, Route<dynamic>? previousRoute) =>
      routes.remove(route);

  @override
  void didReplace({Route<dynamic>? newRoute, Route<dynamic>? oldRoute}) {
    routes[routes.indexOf(oldRoute!)] = newRoute!;
  }

  /// The routes by the names the history gives them: a page by its name, a
  /// dialog as 'modal' and a menu as 'flyout'.
  List<String> get names => [
    for (final route in routes)
      switch (route) {
        NoEffectPageRoute() => route.settings.name!,
        RawDialogRoute() => 'modal',
        _ => 'flyout',
      },
  ];
}

final _routes = _Routes();

Route<Widget> _route(RouteSettings settings) {
  late final NoEffectPageRoute<Widget> route;
  route = NoEffectPageRoute(
    settings: settings,
    builder: (context) => RouterAnimation(_Page(settings.name!), route: route),
  );
  return route;
}

/// The app's navigator, its pages built as the app builds them, under the real
/// navigation bar, with search open.
Future<void> _pumpApp(WidgetTester tester) async {
  tester.view.physicalSize = const Size(1200, 900);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  addTearDown(() {
    _close = _Close.item;
    _navigate = $push;
    _loading = null;
    _trackMenu = false;
  });
  cafeMode = false;
  _routes.routes.clear();
  $router.update('/library', null);
  $history.reset();
  final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
  // No library is open, which the track menu reads.
  final registry = LibraryRegistryProvider(
    loadRecords: () async =>
        const ListLibraryRecordsResponse(records: [], invalidRecords: []),
  );
  addTearDown(registry.dispose);
  final library = LibraryOpenStateProvider(null, registry);
  addTearDown(library.dispose);
  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider<ScreenSizeProvider>.value(value: sizes),
        ChangeNotifierProvider<ResponsiveProvider>(
          create: (_) => ResponsiveProvider(sizes, () async => null),
        ),
        ChangeNotifierProvider<RouterPathProvider>.value(value: $router),
        ChangeNotifierProvider<LibraryOpenStateProvider>.value(value: library),
        ChangeNotifierProvider<LinuxCustomWindowControlsProvider>.value(
          value: LinuxCustomWindowControlsProvider(),
        ),
      ],
      child: FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        navigatorKey: runeWithNavigationBarAndPlaybackControllorNavigatorKey,
        navigatorObservers: [
          routerAnimationObserver,
          FlyoutRouteObserver(),
          _routes,
        ],
        onGenerateRoute: _route,
        onGenerateInitialRoutes: (name) => [_route(RouteSettings(name: name))],
        initialRoute: '/library',
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
  $push(_search);
  await tester.pumpAndSettle();
}

List<String> _names() => [for (final entry in $history.history) entry.name];

_PageState _shownPage(WidgetTester tester) =>
    tester.state<_PageState>(find.byType(_Page));

String? _parent(WidgetTester tester) {
  final link = find.byType(ParentLink);
  return link.evaluate().isEmpty ? null : tester.widget<ParentLink>(link).text;
}

/// Opens the menu of the song on the page shown.
Future<void> _openMenu(WidgetTester tester) async {
  await tester.tap(
    find.byKey(ValueKey('row ${$router.path}')),
    buttons: kSecondaryButton,
  );
  await tester.pumpAndSettle();
}

/// Opens the menu of the song on the page shown and presses [label] in it.
Future<void> _goFromMenu(WidgetTester tester, String label) async {
  await _openMenu(tester);
  expect(_names().last, 'flyout');
  expect(_routes.names.last, 'flyout');
  await tester.tap(find.text(label));
  await tester.pumpAndSettle();
}

/// The pages open, each with its state, kept as the user opens and closes
/// them, to check the app against after every step.
class _Expected {
  _Expected(this.tester, this.searchParent);

  final WidgetTester tester;

  /// What the header names over search, opened from the library.
  final String? searchParent;

  final List<(String, _PageState)> pages = [];

  /// The page shown is the one [name] just opened.
  void opened(String name) {
    pages.add((name, _shownPage(tester)));
    check();
  }

  /// The page shown is the one [name] just opened in place of the page that
  /// was shown.
  void replaced(String name) {
    pages.removeLast();
    opened(name);
  }

  /// The page shown is the one under the page that was just closed, as it
  /// was left.
  void closed() {
    pages.removeLast();
    check();
  }

  /// The page shown is search, as it was left, with every page above it
  /// closed.
  void backToSearch() {
    pages.removeRange(1, pages.length);
    check();
  }

  /// The history holds the pages and nothing else, as the navigator does, and
  /// the page shown, the one under it and the header follow them.
  void check({int menus = 0}) {
    final names = [
      '/library',
      for (final (name, _) in pages) name,
      for (var i = 0; i < menus; i++) 'flyout',
    ];
    expect(_names(), names, reason: 'the history');
    expect(_routes.names, names, reason: "the navigator's routes");
    expect($router.path, pages.last.$1);
    expect(_shownPage(tester), same(pages.last.$2), reason: 'the page shown');
    final shown = ['/library', for (final (name, _) in pages) name];
    expect($history.previousPage?.name, shown[shown.length - 2]);
    expect(_parent(tester), switch (shown) {
      [_, _search] => searchParent,
      [..., _search, _] => 'Search',
      [..., _album, _artist] => 'Artists',
      [..., _artist, _album] => 'Albums',
      _ => fail('no header for $shown'),
    }, reason: 'the header parent');
  }
}

Future<_Expected> _start(WidgetTester tester) async {
  await _pumpApp(tester);
  return _Expected(tester, _parent(tester))..opened(_search);
}

void main() {
  for (final close in _Close.values) {
    testWidgets('pages opened from menus over search and an album keep their '
        'history again and again, the menus ${close.how}', (tester) async {
      final expected = await _start(tester);
      _close = close;

      for (var round = 0; round < 4; round++) {
        await _goFromMenu(tester, 'Go to Album');
        expected.opened(_album);
        await _goFromMenu(tester, 'Go to Artist');
        expected.opened(_artist);

        expect($pop(), isTrue);
        await tester.pumpAndSettle();
        expected.closed();
        navigateBackwardWithPop();
        await tester.pumpAndSettle();
        expected.closed();
      }
    });

    testWidgets('back and the search entry go back through pages '
        'opened from menus, again and again, the menus ${close.how}', (
      tester,
    ) async {
      final expected = await _start(tester);
      _close = close;

      for (var round = 0; round < 3; round++) {
        await _goFromMenu(tester, 'Go to Artist');
        expected.opened(_artist);
        await _goFromMenu(tester, 'Go to Album');
        expected.opened(_album);

        expect($pop(), isTrue);
        await tester.pumpAndSettle();
        expected.closed();
        expect($pop(), isTrue);
        await tester.pumpAndSettle();
        expected.closed();

        await _goFromMenu(tester, 'Go to Album');
        expected.opened(_album);
        await _goFromMenu(tester, 'Go to Artist');
        expected.opened(_artist);
        await _goFromMenu(tester, 'Go to Album');
        expected.opened(_album);

        // As the search button opens it.
        $push(_search);
        await tester.pumpAndSettle();
        expected.backToSearch();
      }
    });
  }

  testWidgets('pages opened from menus a frame apart, before the menus have '
      'finished closing, keep their history', (tester) async {
    final expected = await _start(tester);

    for (var round = 0; round < 3; round++) {
      await _openMenu(tester);
      await tester.tap(find.text('Go to Album'));
      await tester.pump();
      await tester.pump();
      expect(_names(), ['/library', _search, _album]);

      // The album's song is there at once, while the menu over search still
      // fades.
      await tester.tap(
        find.byKey(const ValueKey('row $_album')),
        buttons: kSecondaryButton,
      );
      await tester.pump();
      await tester.pump();
      await tester.tap(find.text('Go to Artist').last);
      await tester.pump();
      await tester.pump();
      expect(_names(), ['/library', _search, _album, _artist]);

      expect($pop(), isTrue);
      await tester.pump();
      expect($pop(), isTrue);
      await tester.pumpAndSettle();
      expected.check();
    }
  });

  testWidgets('back while a menu is open closes the menu and leaves the page '
      'as it was', (tester) async {
    final expected = await _start(tester);

    for (var round = 0; round < 2; round++) {
      await _goFromMenu(tester, 'Go to Album');
      expected.opened(_album);

      // As the mouse's back button and the edge swipe go back.
      await _openMenu(tester);
      expected.check(menus: 1);
      navigateBackwardWithPop();
      await tester.pumpAndSettle();
      expected.check();

      navigateBackwardWithPop();
      await tester.pumpAndSettle();
      expected.closed();
    }
  });

  testWidgets('two menus a song opened at once, as a slow menu can, close as '
      'the pages they open are pushed', (tester) async {
    final expected = await _start(tester);

    for (var round = 0; round < 2; round++) {
      await _goFromMenu(tester, 'Go to Album');
      expected.opened(_album);

      // Pressed twice while its data loads, the song opens two menus from
      // its one controller. A click beside them closes the one on top.
      _loading = Completer<void>();
      for (var i = 0; i < 2; i++) {
        await tester.tap(
          find.byKey(const ValueKey('row $_album')),
          buttons: kSecondaryButton,
        );
        await tester.pump();
      }
      _loading!.complete();
      _loading = null;
      await tester.pumpAndSettle();
      expected.check(menus: 2);
      await tester.tapAt(const Offset(1100, 850));
      await tester.pumpAndSettle();
      expected.check(menus: 1);

      await tester.tap(find.text('Go to Artist'));
      await tester.pumpAndSettle();
      expected.opened(_artist);

      expect($pop(), isTrue);
      await tester.pumpAndSettle();
      expected.closed();
      expect($pop(), isTrue);
      await tester.pumpAndSettle();
      expected.closed();
    }
  });

  testWidgets('a song that goes while its menu is open takes its menu and '
      'its entry with it', (tester) async {
    final expected = await _start(tester);
    await _goFromMenu(tester, 'Go to Album');
    expected.opened(_album);

    for (var round = 0; round < 2; round++) {
      await _openMenu(tester);
      expected.check(menus: 1);

      // The album's list is read again without the song.
      final page = _shownPage(tester);
      // ignore: invalid_use_of_protected_member
      page.setState(() => page.listed = false);
      await tester.pumpAndSettle();
      expected.check();

      // ignore: invalid_use_of_protected_member
      page.setState(() => page.listed = true);
      await tester.pumpAndSettle();
    }

    expect($pop(), isTrue);
    await tester.pumpAndSettle();
    expected.closed();
  });

  testWidgets('a page pushed before a menu is drawn closes the menu', (
    tester,
  ) async {
    final expected = await _start(tester);

    // The menu's route is pushed and not yet built.
    await tester.tap(
      find.byKey(const ValueKey('row $_search')),
      buttons: kSecondaryButton,
    );
    expect(_names().last, 'flyout');
    $push(_album, arguments: QueryTracksParameter(9, 'Nine'));
    await tester.pumpAndSettle();
    expected.opened(_album);

    expect($pop(), isTrue);
    await tester.pumpAndSettle();
    expected.closed();
  });

  testWidgets('a menu let go of before it is drawn closes under a dialog '
      'opened over it', (tester) async {
    final expected = await _start(tester);

    // The menu's route is pushed and not yet built when its owner lets go of
    // it, and a dialog opens over it before it is closed.
    await tester.tap(
      find.byKey(const ValueKey('row $_search')),
      buttons: kSecondaryButton,
    );
    expect(_routes.names.last, 'flyout');
    tester.state<_RowState>(find.byType(_Row))._controller.dispose();
    unawaited(
      $showModal<void>(
        tester.element(find.byType(_Page)),
        (context, close) => const Text('a dialog'),
      ),
    );
    await tester.pumpAndSettle();
    expect(_names(), ['/library', _search, 'modal']);
    expect(_routes.names, ['/library', _search, 'modal']);

    expect($pop(), isTrue);
    await tester.pumpAndSettle();
    expected.check();
  });

  for (final close in _Close.values) {
    testWidgets('Go to from a page opened from search replaces it, again and '
        'again, and back returns to search as it was, the menus ${close.how}', (
      tester,
    ) async {
      final expected = await _start(tester);
      _close = close;
      _navigate = $goTo;
      final search = _shownPage(tester);

      await _goFromMenu(tester, 'Go to Album');
      expected.opened(_album);
      expect(_parent(tester), 'Search');

      // The artist replaces the album, directly above search.
      await _goFromMenu(tester, 'Go to Artist');
      expect(_names(), ['/library', _search, _artist]);
      expect(_routes.names, ['/library', _search, _artist]);
      expect($router.path, _artist);
      expect(_parent(tester), 'Search');

      await _goFromMenu(tester, 'Go to Album');
      await _goFromMenu(tester, 'Go to Artist');
      await _goFromMenu(tester, 'Go to Album');
      expect(_names(), ['/library', _search, _album]);
      expect(_routes.names, ['/library', _search, _album]);
      expect(_parent(tester), 'Search');

      navigateBackwardWithPop();
      await tester.pumpAndSettle();
      expect(_names(), ['/library', _search]);
      expect(_routes.names, ['/library', _search]);
      expect($router.path, _search);
      expect(_shownPage(tester), same(search));
    });
  }

  testWidgets('Go to from a page not opened from search stacks, and back '
      'returns to the page', (tester) async {
    await _pumpApp(tester);
    _navigate = $goTo;
    expect($pop(), isTrue);
    await tester.pumpAndSettle();
    $push(_album, arguments: QueryTracksParameter(9, 'Nine'));
    await tester.pumpAndSettle();
    final album = _shownPage(tester);

    await _goFromMenu(tester, 'Go to Artist');
    expect(_names(), ['/library', _album, _artist]);
    expect(_routes.names, ['/library', _album, _artist]);
    expect(_parent(tester), 'Artists');

    navigateBackwardWithPop();
    await tester.pumpAndSettle();
    expect(_names(), ['/library', _album]);
    expect(_routes.names, ['/library', _album]);
    expect(_shownPage(tester), same(album));
  });

  testWidgets("the track menu's Go to from a page opened from search replaces "
      'it, and back returns to search as it was', (tester) async {
    final expected = await _start(tester);
    _trackMenu = true;

    await _goFromMenu(tester, 'Go to Album');
    expected.opened(_album);
    await _goFromMenu(tester, 'Go to Artist');
    expected.replaced(_artist);

    navigateBackwardWithPop();
    await tester.pumpAndSettle();
    expected.closed();
  });

  testWidgets('a dialog takes its own entry out of the history when it goes, '
      'wherever the entry is', (tester) async {
    $history.reset();
    $history.push(const RouteSettings(name: _search));
    await tester.pumpWidget(
      FluentApp(
        home: ModalRouteWrapper(
          name: 'modal',
          builder: (context, close) => const Text('a dialog'),
        ),
      ),
    );
    expect(_names(), ['/library', _search, 'modal']);

    // A page was pushed over the dialog before it closed.
    $history.push(const RouteSettings(name: _album));
    await tester.pumpWidget(const FluentApp(home: SizedBox()));

    expect(_names(), ['/library', _search, _album]);
  });
}
