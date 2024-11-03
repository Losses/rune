import 'package:fluent_ui/fluent_ui.dart' hide Page;

import '../../utils/router/router_transition_parameter.dart';
import 'utils/transition_calculatior.dart';

class RouterAnimation extends StatefulWidget {
  const RouterAnimation(
    this.child, {
    super.key,
  });

  final Widget child;

  @override
  State<RouterAnimation> createState() => _RouterAnimationState();
}

class _RouterAnimationState extends State<RouterAnimation>
    with TickerProviderStateMixin {
  late AnimationController _animationController;

  String? from;
  String? to;

  @override
  void initState() {
    super.initState();
    _animationController = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 300),
    );
    final transition = getRouterTransitionParameter();
    if (transition != null) {
      from = transition.from;
      to = transition.to;
    }
  }

  late RouteRelation relation;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();

    relation = $transition.compareRoute(from, to);
    _animationController.reset();
    _animationController.forward();
  }

  @override
  void dispose() {
    _animationController.dispose();
    super.dispose();
  }

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

    return _applyAnimation(widget.child, relation);
  }
}
