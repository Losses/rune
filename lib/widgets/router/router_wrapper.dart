import 'dart:io';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart' hide Page;

import '../../utils/router/router_transition_parameter.dart';
import '../../config/theme.dart';
import '../../providers/transition_calculation.dart';

import '../shortcuts/router_actions_manager.dart';

class $ extends StatefulWidget {
  const $(
    this.child, {
    super.key,
  });

  final Widget child;

  @override
  State<$> createState() => _$State();
}

class _$State extends State<$> with TickerProviderStateMixin {
  late AnimationController _animationController;

  String? from;
  String? to;

  void _updateWindowEffectCallback() {
    if (Platform.isLinux) return;
    if (Platform.isAndroid) return;
  }

  void updateWindowEffect() {
    if (Platform.isLinux) return;
    if (Platform.isAndroid) return;

    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) {
        appTheme.setEffect(context);
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
    appTheme.addListener(_updateWindowEffectCallback);
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _updateWindowEffectCallback();
    final transition = getRouterTransitionParameter();
    if (transition != null) {
      from = transition.from;
      to = transition.to;
    }
  }

  @override
  void dispose() {
    _animationController.dispose();
    appTheme.removeListener(_updateWindowEffectCallback);
    super.dispose();
  }

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

    if (from == to) {
      return _applyAnimation(widget.child, _lastCompareResult);
    }

    final relation = calculator.compareRoute(from, to);
    _lastCompareResult = relation;
    _animationController.reset();
    _animationController.forward();

    return NavigationShortcutManager(
      child: _applyAnimation(widget.child, relation),
    );
  }
}
