import 'dart:io';

import 'package:rinf/rinf.dart';
import 'package:provider/provider.dart';
import 'package:flutter/foundation.dart';
import 'package:go_router/go_router.dart';
import 'package:get_storage/get_storage.dart';
import 'package:system_theme/system_theme.dart';
import 'package:fluent_ui/fluent_ui.dart' hide Page;
import 'package:window_manager/window_manager.dart';
import 'package:flutter_acrylic/flutter_acrylic.dart' as flutter_acrylic;

import 'config/theme.dart';
import 'config/routes.dart';
import 'config/app_title.dart';
import 'config/navigation.dart';

import 'providers/status.dart';
import 'providers/playlist.dart';
import 'providers/library_path.dart';
import 'providers/library_manager.dart';

import 'widgets/flip_animation.dart';
import 'widgets/navigation_bar.dart';
import 'widgets/playback_controller.dart';

import 'utils/platform.dart';

import 'routes/welcome.dart' as welcome;

import 'messages/generated.dart';

import 'theme.dart';

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
        ChangeNotifierProvider(create: (_) => LibraryManagerProvider()),
      ],
      child: const Rune(),
    ),
  );
}

class Rune extends StatelessWidget {
  const Rune({super.key});

  @override
  Widget build(BuildContext context) {
    PlaylistUpdateHandler.init(context);
    PlaybackStatusUpdateHandler.init(context);

    return ChangeNotifierProvider.value(
      value: appTheme,
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
          routeInformationParser: router.routeInformationParser,
          routerDelegate: router.routerDelegate,
          routeInformationProvider: router.routeInformationProvider,
          builder: (context, child) {
            return Directionality(
              textDirection: appTheme.textDirection,
              child: child!,
            );
          },
        );
      },
    );
  }
}

class RouterFrame extends StatefulWidget {
  final AppTheme appTheme;

  const RouterFrame({
    super.key,
    required this.child,
    required this.shellContext,
    required this.appTheme,
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
    super.initState();
    widget.appTheme.addListener(_updateWindowEffectCallback);
  }

  void _updateWindowEffectCallback() {
    final theme = FluentTheme.of(context);
    updateWindowEffect(theme);
  }

  void updateWindowEffect(FluentThemeData theme) {
    if (Platform.isLinux) return;

    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) {
        widget.appTheme.setEffect(theme);
      }
    });
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final theme = FluentTheme.of(context);
    updateWindowEffect(theme);
  }

  @override
  void dispose() {
    widget.appTheme.removeListener(_updateWindowEffectCallback);
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

final rootNavigatorKey = GlobalKey<NavigatorState>();
final _shellNavigatorKey = GlobalKey<NavigatorState>();
final router = GoRouter(
    navigatorKey: rootNavigatorKey,
    initialLocation: "/library",
    routes: [
      ShellRoute(
        navigatorKey: _shellNavigatorKey,
        builder: (context, state, child) {
          final library = Provider.of<LibraryPathProvider>(context);
          final theme = FluentTheme.of(context);

          if (library.currentPath == null) {
            return const welcome.WelcomePage();
          }

          if (library.scanning) {
            return const welcome.ScanningPage();
          }

          return Container(
            color: Platform.isLinux
                ? theme.micaBackgroundColor
                : Colors.transparent,
            child: FlipAnimationContext(
                child: Stack(alignment: Alignment.bottomCenter, children: [
              SizedBox.expand(
                child: RouterFrame(
                  shellContext: _shellNavigatorKey.currentContext,
                  appTheme: appTheme,
                  child: child,
                ),
              ),
              const PlaybackController(),
              NavigationBar(items: navigationItems),
            ])),
          );
        },
        routes: routes,
      ),
    ]);
