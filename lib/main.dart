import 'package:player/widgets/flip_animation.dart';
import 'package:rinf/rinf.dart';
import 'package:url_launcher/link.dart';
import 'package:provider/provider.dart';
import 'package:flutter/foundation.dart';
import 'package:go_router/go_router.dart';
import 'package:get_storage/get_storage.dart';
import 'package:system_theme/system_theme.dart';
import 'package:fluent_ui/fluent_ui.dart' hide Page;
import 'package:window_manager/window_manager.dart';
import 'package:flutter_acrylic/window_effect.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:flutter_acrylic/flutter_acrylic.dart' as flutter_acrylic;

import 'routes/home.dart' deferred as home;
import 'routes/tracks.dart' deferred as tracks;
import 'routes/albums.dart' deferred as albums;
import 'routes/artists.dart' deferred as artists;
import 'routes/settings.dart' deferred as settings;
import 'routes/cover_wall.dart' deferred as cover_wall;
import 'routes/query_tracks.dart' deferred as query_tracks;

import 'providers/status.dart';
import 'providers/playlist.dart';
import 'providers/library_path.dart';

import 'widgets/navigation_bar.dart';
import 'widgets/theme_gradient.dart';
import 'widgets/deferred_widget.dart';
import 'widgets/playback_controller.dart';

import 'utils/attach_navigation_event.dart';
import 'utils/navigation_indicator_helper.dart';

import 'theme.dart';
import 'messages/generated.dart';

const String appTitle = 'Rune Player';

/// Checks if the current environment is a desktop environment.
bool get isDesktop {
  if (kIsWeb) return false;
  return [
    TargetPlatform.windows,
    TargetPlatform.linux,
    TargetPlatform.macOS,
  ].contains(defaultTargetPlatform);
}

void main() async {
  await GetStorage.init();
  await initializeRust(assignRustSignal);
  WidgetsFlutterBinding.ensureInitialized();

  // if it's not on the web, windows or android, load the accent color
  if (!kIsWeb &&
      [
        TargetPlatform.windows,
        TargetPlatform.android,
      ].contains(defaultTargetPlatform)) {
    SystemTheme.accentColor.load();
  }

  if (isDesktop) {
    await flutter_acrylic.Window.initialize();
    await WindowManager.instance.ensureInitialized();
    windowManager.waitUntilReadyToShow().then((_) async {
      await windowManager.setMinimumSize(const Size(500, 600));
      await windowManager.show();
      await windowManager.setPreventClose(true);
    });
  }

  Future.wait([
    DeferredWidget.preload(home.loadLibrary),
  ]);

  runApp(
    MultiProvider(
      providers: [
        ChangeNotifierProvider(create: (_) => LibraryPathProvider()),
        ChangeNotifierProvider(create: (_) => PlaylistProvider()),
        ChangeNotifierProvider(create: (_) => PlaybackStatusProvider()),
      ],
      child: const MyApp(),
    ),
  );
}

final _appTheme = AppTheme();

class MyApp extends StatelessWidget {
  const MyApp({super.key});

  @override
  Widget build(BuildContext context) {
    PlaylistUpdateHandler.init(context);
    PlaybackStatusUpdateHandler.init(context);

    return ChangeNotifierProvider.value(
      value: _appTheme,
      builder: (context, child) {
        final appTheme = context.watch<AppTheme>();

        return FluentApp.router(
          title: appTitle,
          themeMode: appTheme.mode,
          debugShowCheckedModeBanner: false,
          color: appTheme.color,
          darkTheme: FluentThemeData(
            brightness: Brightness.dark,
            accentColor: appTheme.color,
            visualDensity: VisualDensity.standard,
            focusTheme: FocusThemeData(
              glowFactor: is10footScreen(context) ? 2.0 : 0.0,
            ),
          ),
          theme: FluentThemeData(
            accentColor: appTheme.color,
            visualDensity: VisualDensity.standard,
            focusTheme: FocusThemeData(
              glowFactor: is10footScreen(context) ? 2.0 : 0.0,
            ),
          ),
          locale: appTheme.locale,
          builder: (context, child) {
            WidgetsBinding.instance.addPostFrameCallback((_) {
              appTheme.setEffect(context);
            });

            return Directionality(
              textDirection: appTheme.textDirection,
              child: NavigationPaneTheme(
                data: NavigationPaneThemeData(
                  backgroundColor: appTheme.windowEffect == WindowEffect.mica
                      ? Colors.transparent
                      : FluentTheme.of(context).brightness == Brightness.light
                          ? const Color(0xFFF6F6F6)
                          : const Color(0xFF1F1F1F),
                ),
                child: child!,
              ),
            );
          },
          routeInformationParser: router.routeInformationParser,
          routerDelegate: router.routerDelegate,
          routeInformationProvider: router.routeInformationProvider,
        );
      },
    );
  }
}

class MyHomePage extends StatefulWidget {
  const MyHomePage({
    super.key,
    required this.child,
    required this.shellContext,
  });

  final Widget child;
  final BuildContext? shellContext;

  @override
  State<MyHomePage> createState() => _MyHomePageState();
}

class _MyHomePageState extends State<MyHomePage> with WindowListener {
  bool value = false;

  final viewKey = GlobalKey(debugLabel: 'Navigation View Key');

  late final originalItems = attachNavigationEvent(context, [
    PaneItem(
      key: const ValueKey('/home'),
      icon: const Icon(Symbols.home),
      title: const Text('Home'),
      body: const SizedBox.shrink(),
    ),
    PaneItem(
      key: const ValueKey('/artists'),
      icon: const Icon(Symbols.face),
      title: const Text('artists'),
      body: const SizedBox.shrink(),
    ),
    PaneItem(
      key: const ValueKey('/albums'),
      icon: const Icon(Symbols.album),
      title: const Text('albums'),
      body: const SizedBox.shrink(),
    ),
    PaneItem(
      key: const ValueKey('/tracks'),
      icon: const Icon(Symbols.library_music),
      title: const Text('Tracks'),
      body: const SizedBox.shrink(),
    ),
  ]);

  late final footerItems = attachNavigationEvent(context, [
    PaneItem(
      key: const ValueKey('/settings'),
      icon: const Icon(Symbols.settings),
      title: const Text('Settings'),
      body: const SizedBox.shrink(),
    ),
  ]);

  late final NavigationIndicatorHelper navigationIndicatorHelper =
      NavigationIndicatorHelper(originalItems, footerItems, routes);

  @override
  void initState() {
    windowManager.addListener(this);
    super.initState();
  }

  @override
  void dispose() {
    windowManager.removeListener(this);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    FluentLocalizations.of(context);

    final appTheme = context.watch<AppTheme>();
    if (widget.shellContext != null) {
      if (router.canPop() == false) {
        setState(() {});
      }
    }

    final routeState = GoRouterState.of(context);

    final navigation = NavigationView(
      key: viewKey,
      paneBodyBuilder: (item, child) {
        final name =
            item?.key is ValueKey ? (item!.key as ValueKey).value : null;
        return FocusTraversalGroup(
          key: ValueKey('body$name'),
          child: widget.child,
        );
      },
      pane: NavigationPane(
        selected: navigationIndicatorHelper.calculateSelectedIndex(context),
        header: const SizedBox(
          height: kOneLineTileHeight,
          child: ThemeGradient(),
        ),
        displayMode: routeState.fullPath != "/cover_wall"
            ? appTheme.displayMode
            : PaneDisplayMode.minimal,
        indicator: const StickyNavigationIndicator(),
        items: originalItems,
        footerItems: footerItems,
      ),
    );

    return FlipAnimationContext(
        child: Stack(alignment: Alignment.bottomCenter, children: [
      SizedBox.expand(
        child: navigation,
      ),
      const PlaybackController(),
      NavigationBar(items: navigationItems),
    ]));
  }

  @override
  void onWindowClose() async {
    bool isPreventClose = await windowManager.isPreventClose();
    if (isPreventClose && mounted) {
      showDialog(
        context: context,
        builder: (_) {
          return ContentDialog(
            title: const Text('Confirm close'),
            content: const Text('Are you sure you want to close this window?'),
            actions: [
              FilledButton(
                child: const Text('Yes'),
                onPressed: () {
                  Navigator.pop(context);
                  windowManager.destroy();
                },
              ),
              Button(
                child: const Text('No'),
                onPressed: () {
                  Navigator.pop(context);
                },
              ),
            ],
          );
        },
      );
    }
  }
}

class WindowButtons extends StatelessWidget {
  const WindowButtons({super.key});

  @override
  Widget build(BuildContext context) {
    final FluentThemeData theme = FluentTheme.of(context);

    return SizedBox(
      width: 138,
      height: 50,
      child: WindowCaption(
        brightness: theme.brightness,
        backgroundColor: Colors.transparent,
      ),
    );
  }
}

class LinkPaneItemAction extends PaneItem {
  LinkPaneItemAction({
    required super.icon,
    required this.link,
    required super.body,
    super.title,
  });

  final String link;

  @override
  Widget build(
    BuildContext context,
    bool selected,
    VoidCallback? onPressed, {
    PaneDisplayMode? displayMode,
    bool showTextOnTop = true,
    bool? autofocus,
    int? itemIndex,
  }) {
    return Link(
      uri: Uri.parse(link),
      builder: (context, followLink) => Semantics(
        link: true,
        child: super.build(
          context,
          selected,
          followLink,
          displayMode: displayMode,
          showTextOnTop: showTextOnTop,
          itemIndex: itemIndex,
          autofocus: autofocus,
        ),
      ),
    );
  }
}

final routes = <GoRoute>[
  GoRoute(
    path: '/home',
    builder: (context, state) => DeferredWidget(
      home.loadLibrary,
      () => home.HomePage(),
    ),
  ),
  GoRoute(
    path: '/library',
    builder: (context, state) => DeferredWidget(
      home.loadLibrary,
      () => home.HomePage(),
    ),
  ),
  GoRoute(
    path: '/artists',
    builder: (context, state) => DeferredWidget(
      artists.loadLibrary,
      () => artists.ArtistsPage(),
    ),
  ),
  GoRoute(
    path: '/artists/:artistId',
    builder: (context, state) => DeferredWidget(
      query_tracks.loadLibrary,
      () => query_tracks.QueryTracksPage(
        artistIds: [int.parse(state.pathParameters['artistId'] ?? "0")],
      ),
    ),
  ),
  GoRoute(
    path: '/albums',
    builder: (context, state) => DeferredWidget(
      albums.loadLibrary,
      () => albums.AlbumsPage(),
    ),
  ),
  GoRoute(
    path: '/albums/:albumId',
    builder: (context, state) => DeferredWidget(
      query_tracks.loadLibrary,
      () => query_tracks.QueryTracksPage(
        albumIds: [int.parse(state.pathParameters['albumId'] ?? "0")],
      ),
    ),
  ),
  GoRoute(
    path: '/tracks',
    builder: (context, state) => DeferredWidget(
      tracks.loadLibrary,
      () => tracks.TracksPage(),
    ),
  ),
  GoRoute(
    path: '/settings',
    builder: (context, state) => DeferredWidget(
      settings.loadLibrary,
      () => settings.SettingsPage(),
    ),
  ),
  GoRoute(
    path: '/cover_wall',
    builder: (context, state) => DeferredWidget(
      cover_wall.loadLibrary,
      () => cover_wall.CoverWallPage(),
    ),
  ),
];

final rootNavigatorKey = GlobalKey<NavigatorState>();
final _shellNavigatorKey = GlobalKey<NavigatorState>();
final router =
    GoRouter(navigatorKey: rootNavigatorKey, initialLocation: "/home", routes: [
  ShellRoute(
    navigatorKey: _shellNavigatorKey,
    builder: (context, state, child) {
      return MyHomePage(
        shellContext: _shellNavigatorKey.currentContext,
        child: child,
      );
    },
    routes: routes,
  ),
]);

final List<NavigationItem> navigationItems = [
  NavigationItem('Home', '/home', children: [
    NavigationItem('Library', '/library', children: [
      NavigationItem('Artists', '/artists', children: [
        NavigationItem('Artist Query', '/artists/:artistId', hidden: true),
      ]),
      NavigationItem('Albums', '/albums', children: [
        NavigationItem('Artist Query', '/albums/:albumId', hidden: true),
      ]),
    ]),
    NavigationItem('Settings', '/settings'),
  ]),
];
