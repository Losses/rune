import 'package:fluent_ui/fluent_ui.dart' hide Page;
import 'package:flutter/foundation.dart';
import 'package:flutter_acrylic/flutter_acrylic.dart' as flutter_acrylic;
import 'package:flutter_acrylic/window_effect.dart';
import 'package:get_storage/get_storage.dart';
import 'package:go_router/go_router.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:provider/provider.dart';
import 'package:system_theme/system_theme.dart';
import 'package:url_launcher/link.dart';
import 'package:window_manager/window_manager.dart';
import 'package:rinf/rinf.dart';

import 'routes/home.dart' deferred as home;
import 'routes/tracks.dart' deferred as tracks;
import 'routes/artists.dart' deferred as artists;
import 'routes/settings.dart' deferred as settings;
import 'routes/cover_wall.dart' deferred as cover_wall;

import 'providers/library_path.dart';
import 'providers/playlist.dart';
import 'providers/status.dart';

import 'widgets/theme_gradient.dart';
import 'widgets/deferred_widget.dart';
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

  late final List<NavigationPaneItem> originalItems = [
    PaneItem(
      key: const ValueKey('/'),
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
      key: const ValueKey('/tracks'),
      icon: const Icon(Symbols.library_music),
      title: const Text('Tracks'),
      body: const SizedBox.shrink(),
    ),
  ].map<NavigationPaneItem>((e) {
    PaneItem buildPaneItem(PaneItem item) {
      return PaneItem(
        key: item.key,
        icon: item.icon,
        title: item.title,
        body: item.body,
        onTap: () {
          final path = (item.key as ValueKey).value;
          if (GoRouterState.of(context).uri.toString() != path) {
            context.push(path);
          }
          item.onTap?.call();
        },
      );
    }

    if (e is PaneItemExpander) {
      return PaneItemExpander(
        key: e.key,
        icon: e.icon,
        title: e.title,
        body: e.body,
        items: e.items.map((item) {
          if (item is PaneItem) return buildPaneItem(item);
          return item;
        }).toList(),
      );
    }
    return buildPaneItem(e);
  }).toList();
  late final List<NavigationPaneItem> footerItems = [
    PaneItem(
      key: const ValueKey('/settings'),
      icon: const Icon(Symbols.settings),
      title: const Text('Settings'),
      body: const SizedBox.shrink(),
      onTap: () {
        if (GoRouterState.of(context).uri.toString() != '/settings') {
          context.push('/settings');
        }
      },
    ),
  ];

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

  int _calculateSelectedIndex(BuildContext context) {
    final location = GoRouterState.of(context).uri.toString();
    int indexOriginal = originalItems
        .where((item) => item.key != null)
        .toList()
        .indexWhere((item) => item.key == Key(location));

    if (indexOriginal == -1) {
      int indexFooter = footerItems
          .where((element) => element.key != null)
          .toList()
          .indexWhere((element) => element.key == Key(location));
      if (indexFooter == -1) {
        return 0;
      }
      return originalItems
              .where((element) => element.key != null)
              .toList()
              .length +
          indexFooter;
    } else {
      return indexOriginal;
    }
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
        selected: _calculateSelectedIndex(context),
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

    return Stack(alignment: Alignment.bottomCenter, children: [
      SizedBox.expand(
        child: navigation,
      ),
      const PlaybackController(),
    ]);
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

final rootNavigatorKey = GlobalKey<NavigatorState>();
final _shellNavigatorKey = GlobalKey<NavigatorState>();
final router = GoRouter(navigatorKey: rootNavigatorKey, routes: [
  ShellRoute(
    navigatorKey: _shellNavigatorKey,
    builder: (context, state, child) {
      return MyHomePage(
        shellContext: _shellNavigatorKey.currentContext,
        child: child,
      );
    },
    routes: <GoRoute>[
      GoRoute(
        path: '/',
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
    ],
  ),
]);
