import 'dart:io';

import 'package:provider/provider.dart';
import 'package:go_router/go_router.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:fluent_ui/fluent_ui.dart' hide Page;

import 'utils/navigation/utils/navigation_backward.dart';

import 'config/theme.dart';
import 'config/routes.dart';

import 'routes/welcome.dart' as welcome;

import 'widgets/ax_pressure.dart';
import 'widgets/hover_opacity.dart';
import 'widgets/navigation_bar/flip_animation.dart';
import 'widgets/navigation_bar/navigation_bar.dart';
import 'widgets/shortcuts/router_actions_manager.dart';
import 'widgets/playback_controller/playback_controller.dart';

import 'providers/library_path.dart';
import 'providers/responsive_providers.dart';
import 'providers/transition_calculation.dart';

import 'theme.dart';

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
  late AnimationController _animationController;

  void _updateWindowEffectCallback() {
    if (Platform.isLinux) return;

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
  void initState() {
    super.initState();
    _animationController = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 300),
    );
    widget.appTheme.addListener(_updateWindowEffectCallback);
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _updateWindowEffectCallback();
  }

  @override
  void dispose() {
    _animationController.dispose();
    widget.appTheme.removeListener(_updateWindowEffectCallback);
    super.dispose();
  }

  String _lastRoute = '';
  RouteRelation _lastCompareResult = RouteRelation.same;

  Widget _applyAnimation(Widget child, RouteRelation relation) {
    const distance = 0.1;
    const curve = Curves.easeOutQuint;

    Animation<Offset> createSlideAnimation(Offset begin) {
      return Tween<Offset>(begin: begin, end: Offset.zero).animate(
        CurvedAnimation(
          parent: _animationController,
          curve: curve,
        ),
      );
    }

    Animation<double> createFadeAnimation() {
      return Tween<double>(begin: 0, end: 1).animate(
        CurvedAnimation(
          parent: _animationController,
          curve: curve,
        ),
      );
    }

    Animation<double> createScaleAnimation() {
      return Tween<double>(begin: 1.1, end: 1).animate(
        CurvedAnimation(
          parent: _animationController,
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

    final calculator = Provider.of<TransitionCalculationProvider>(context);
    final path = GoRouterState.of(context).fullPath ?? "/";

    if (path == _lastRoute) {
      return _applyAnimation(widget.child, _lastCompareResult);
    }

    final relation = calculator.compareRoute(path);
    calculator.registerRoute(path);
    _lastRoute = path;
    _lastCompareResult = relation;
    _animationController.reset();
    _animationController.forward();

    return NavigationShortcutManager(
      child: _applyAnimation(widget.child, relation),
    );
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

        return FlipAnimationContext(
          child: Stack(
            alignment: Alignment.bottomCenter,
            children: [
              SizedBox.expand(
                child: RouterFrame(
                  shellContext: _shellNavigatorKey.currentContext,
                  appTheme: appTheme,
                  child: child,
                ),
              ),
              const PlaybackController(),
              SmallerOrEqualTo(
                breakpoint: DeviceType.band,
                builder: (context, isBand) {
                  if (!isBand) return const NavigationBar();

                  return const BackButton();
                },
              ),
            ],
          ),
        );
      },
      routes: routes,
    ),
  ],
);

class BackButton extends StatefulWidget {
  const BackButton({
    super.key,
  });

  @override
  State<BackButton> createState() => _BackButtonState();
}

class _BackButtonState extends State<BackButton> {
  final FocusNode _focusNode = FocusNode();

  @override
  void dispose() {
    _focusNode.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final routerState = GoRouterState.of(context);

    if (routerState.fullPath == '/library') {
      return Container();
    }

    return Positioned(
      top: -12,
      left: -12,
      child: AxPressure(
        child: GestureDetector(
          onTap: () {
            navigateBackwardWithPop(context);
          },
          child: HoverOpacity(
            child: FocusableActionDetector(
              focusNode: _focusNode,
              child: SvgPicture.asset(
                'assets/arrow-circle-left-solid.svg',
                width: 56,
                colorFilter: ColorFilter.mode(
                  FluentTheme.of(context).inactiveColor,
                  BlendMode.srcIn,
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
