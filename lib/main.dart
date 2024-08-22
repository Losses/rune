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
import 'package:flutter_acrylic/flutter_acrylic.dart' as flutter_acrylic;

import 'routes/home.dart' as home;
import 'routes/tracks.dart' as tracks;
import 'routes/albums.dart' as albums;
import 'routes/artists.dart' as artists;
import 'routes/settings.dart' as settings;
import 'routes/playlists.dart' as playlists;
import 'routes/cover_wall.dart' as cover_wall;
import 'routes/query_tracks.dart' as query_tracks;
import 'routes/library_home.dart' as library_home;

import 'providers/status.dart';
import 'providers/playlist.dart';
import 'providers/library_path.dart';

import 'widgets/flip_animation.dart';
import 'widgets/navigation_bar.dart';
import 'widgets/playback_controller.dart';

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

  runApp(
    MultiProvider(
      providers: [
        ChangeNotifierProvider(
            lazy: false, create: (_) => LibraryPathProvider()),
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

class RouterFrame extends StatefulWidget {
  const RouterFrame({
    super.key,
    required this.child,
    required this.shellContext,
  });

  final Widget child;
  final BuildContext? shellContext;

  @override
  State<RouterFrame> createState() => _RouterFrameState();
}

class _RouterFrameState extends State<RouterFrame> with WindowListener {
  bool value = false;

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

    if (widget.shellContext != null) {
      if (router.canPop() == false) {
        setState(() {});
      }
    }

    return widget.child;
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
    builder: (context, state) => const home.HomePage(),
  ),
  GoRoute(
    path: '/library',
    builder: (context, state) => const library_home.LibraryHomePage(),
  ),
  GoRoute(
    path: '/artists',
    builder: (context, state) => const artists.ArtistsPage(),
  ),
  GoRoute(
    path: '/artists/:artistId',
    builder: (context, state) => query_tracks.QueryTracksPage(
      artistIds: [int.parse(state.pathParameters['artistId'] ?? "0")],
    ),
  ),
  GoRoute(
    path: '/albums',
    builder: (context, state) => const albums.AlbumsPage(),
  ),
  GoRoute(
    path: '/albums/:albumId',
    builder: (context, state) => query_tracks.QueryTracksPage(
      albumIds: [int.parse(state.pathParameters['albumId'] ?? "0")],
    ),
  ),
  GoRoute(
    path: '/playlists',
    builder: (context, state) => const playlists.PlaylistsPage(),
  ),
  GoRoute(
    path: '/playlists/:playlistsId',
    builder: (context, state) => query_tracks.QueryTracksPage(
      playlistIds: [int.parse(state.pathParameters['playlistsId'] ?? "0")],
    ),
  ),
  GoRoute(
    path: '/tracks',
    builder: (context, state) => const tracks.TracksPage(),
  ),
  GoRoute(
    path: '/settings',
    builder: (context, state) => const settings.SettingsPage(),
  ),
  GoRoute(
    path: '/cover_wall',
    builder: (context, state) => const cover_wall.CoverWallPage(),
  ),
];

final rootNavigatorKey = GlobalKey<NavigatorState>();
final _shellNavigatorKey = GlobalKey<NavigatorState>();
final router = GoRouter(
    navigatorKey: rootNavigatorKey,
    initialLocation: "/library",
    routes: [
      ShellRoute(
        navigatorKey: _shellNavigatorKey,
        builder: (context, state, child) {
          return FlipAnimationContext(
              child: Stack(alignment: Alignment.bottomCenter, children: [
            SizedBox.expand(
              child: RouterFrame(
                shellContext: _shellNavigatorKey.currentContext,
                child: child,
              ),
            ),
            const PlaybackController(),
            NavigationBar(items: navigationItems),
          ]));
        },
        routes: routes,
      ),
    ]);

final List<NavigationItem> navigationItems = [
  NavigationItem('Rune', '/home', tappable: false, children: [
    NavigationItem('Library', '/library', children: [
      NavigationItem('Artists', '/artists', children: [
        NavigationItem('Artist Query', '/artists/:artistId', hidden: true),
      ]),
      NavigationItem('Albums', '/albums', children: [
        NavigationItem('Artist Query', '/albums/:albumId', hidden: true),
      ]),
      NavigationItem('Playlists', '/playlists', children: [
        NavigationItem('Playlist Query', '/playlists/:albumId', hidden: true),
      ]),
      NavigationItem('Tracks', '/tracks'),
    ]),
    NavigationItem('Settings', '/settings'),
  ]),
];
