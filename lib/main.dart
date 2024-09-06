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

import 'utils/platform.dart';

import 'config/theme.dart';
import 'config/routes.dart';
import 'config/app_title.dart';
import 'config/navigation.dart';

import 'routes/welcome.dart' as welcome;

import 'widgets/flip_animation.dart';
import 'widgets/navigation_bar.dart';
import 'widgets/playback_controller.dart';

import 'messages/generated.dart';

import 'providers/status.dart';
import 'providers/playlist.dart';
import 'providers/library_path.dart';
import 'providers/library_manager.dart';
import 'providers/transition_calculation.dart';

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
      await windowManager.setTitle(appTitle);
      await windowManager.show();
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
        ChangeNotifierProvider(
            create: (_) =>
                TransitionCalculationProvider(navigationItems: navigationItems))
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
            final theme = FluentTheme.of(context);

            return Container(
                color: Platform.isLinux
                    ? theme.micaBackgroundColor
                    : Colors.transparent,
                child: Directionality(
                  textDirection: appTheme.textDirection,
                  child: child!,
                ));
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

class _RouterFrameState extends State<RouterFrame>
    with TickerProviderStateMixin {
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

  Widget _applyAnimation(Widget child, RouteRelation relation) {
    const distance = 0.1;
    const duration = 300;
    const curve = Curves.easeOutQuint;

    AnimationController createController() {
      return AnimationController(
        vsync: this,
        duration: const Duration(milliseconds: duration),
      )..forward();
    }

    Animation<Offset> createSlideAnimation(Offset begin) {
      return Tween<Offset>(begin: begin, end: Offset.zero).animate(
        CurvedAnimation(
          parent: createController(),
          curve: curve,
        ),
      );
    }

    Animation<double> createFadeAnimation() {
      return Tween<double>(begin: 0, end: 1).animate(
        CurvedAnimation(
          parent: createController(),
          curve: curve,
        ),
      );
    }

    Animation<double> createScaleAnimation() {
      return Tween<double>(begin: 1.1, end: 1).animate(
        CurvedAnimation(
          parent: createController(),
          curve: curve,
        ),
      );
    }

    Widget applySlideAndFade(Offset begin) {
      return SlideTransition(
        position: createSlideAnimation(begin),
        child: FadeTransition(
          opacity: createFadeAnimation(),
          child: child,
        ),
      );
    }

    switch (relation) {
      case RouteRelation.parent:
        return applySlideAndFade(const Offset(0, -distance));
      case RouteRelation.child:
        return applySlideAndFade(const Offset(0, distance));
      case RouteRelation.sameLevelAhead:
        return SlideTransition(
          position: createSlideAnimation(const Offset(-distance, 0)),
          child: child,
        );
      case RouteRelation.sameLevelBehind:
        return SlideTransition(
          position: createSlideAnimation(const Offset(distance, 0)),
          child: child,
        );
      case RouteRelation.same:
        return child;
      case RouteRelation.crossLevel:
        return ScaleTransition(
          scale: createScaleAnimation(),
          child: FadeTransition(
            opacity: createFadeAnimation(),
            child: child,
          ),
        );
      default:
        return child;
    }
  }

  @override
  Widget build(BuildContext context) {
    FluentLocalizations.of(context);

    if (widget.shellContext != null) {
      if (Navigator.of(context).canPop() == false) {
        setState(() {});
      }
    }

    final calculator = Provider.of<TransitionCalculationProvider>(context);
    final path = GoRouterState.of(context).fullPath ?? "/";

    final relation = calculator.compareRoute(path);
    calculator.registerRoute(path);

    return _applyAnimation(widget.child, relation);
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

        if (library.currentPath == null) {
          return const welcome.WelcomePage();
        }

        if (library.scanning) {
          return const welcome.ScanningPage();
        }

        final isSearch = state.fullPath == '/search';

        return FlipAnimationContext(
          child: Stack(alignment: Alignment.bottomCenter, children: [
            SizedBox.expand(
              child: Padding(
                padding: EdgeInsets.only(top: isSearch ? 0 : 40),
                child: RouterFrame(
                  shellContext: _shellNavigatorKey.currentContext,
                  appTheme: appTheme,
                  child: child,
                ),
              ),
            ),
            const PlaybackController(),
            NavigationBar(items: navigationItems),
          ]),
        );
      },
      routes: routes,
    ),
  ],
);
