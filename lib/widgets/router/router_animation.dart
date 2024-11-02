import 'dart:io';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart' hide Page;

import '../shortcuts/router_actions_manager.dart';

import '../../providers/transition_calculation.dart';

import '../../theme.dart';

class RouterAnimation extends StatefulWidget {
  final AppTheme appTheme;

  const RouterAnimation({
    super.key,
    required this.child,
    required this.appTheme,
  });

  final Widget child;

  @override
  State<RouterAnimation> createState() => _RouterAnimationState();
}

class _RouterAnimationState extends State<RouterAnimation>
    with TickerProviderStateMixin {
  late AnimationController _animationController;

  void _updateWindowEffectCallback() {
    if (Platform.isLinux) return;
    if (Platform.isAndroid) return;
  }

  void updateWindowEffect() {
    if (Platform.isLinux) return;
    if (Platform.isAndroid) return;

    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) {
        widget.appTheme.setEffect(context);
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
    final path = ModalRoute.of(context)?.settings.name ?? "/";

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
