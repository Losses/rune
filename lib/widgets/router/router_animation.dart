import 'package:fluent_ui/fluent_ui.dart' hide Page;

import '../../utils/router/router_transition_parameter.dart';
import 'utils/transition_calculatior.dart';

/// Tells the pages of the library navigator when a page above them is popped.
final RouteObserver<ModalRoute<void>> routerAnimationObserver =
    RouteObserver<ModalRoute<void>>();

class RouterAnimation extends StatefulWidget {
  const RouterAnimation(
    this.child, {
    this.route,
    super.key,
  });

  final Widget child;

  /// The route this page is the content of, to hear when a page above it is
  /// popped. Looked up from the context it was not given, which would also
  /// rebuild the page whenever a dialog opens over it.
  final ModalRoute<void>? route;

  @override
  State<RouterAnimation> createState() => _RouterAnimationState();
}

class _RouterAnimationState extends State<RouterAnimation>
    with TickerProviderStateMixin, RouteAware {
  late AnimationController _animationController;

  String? from;
  String? to;

  // The transition this page last played, to tell a new one from the same.
  RouterTransitionParameter? _animatedWith;
  bool _subscribed = false;

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
    _animatedWith = transition;
    _subscribe();
  }

  @override
  void didUpdateWidget(covariant RouterAnimation oldWidget) {
    super.didUpdateWidget(oldWidget);
    _subscribe();
  }

  void _subscribe() {
    final route = widget.route;
    if (_subscribed || route == null) return;
    routerAnimationObserver.subscribe(this, route);
    _subscribed = true;
  }

  late RouteRelation relation;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();

    relation = $transition.compareRoute(from, to);
    _animationController.reset();
    _animationController.forward();
  }

  /// A page above this one was popped. When that changed the router's
  /// transition, play the entrance a newly opened page would. A dialog closing
  /// leaves the router alone and so does not replay.
  @override
  void didPopNext() {
    final transition = getRouterTransitionParameter();
    if (transition == null || identical(transition, _animatedWith)) return;
    _animatedWith = transition;
    setState(() {
      from = transition.from;
      to = transition.to;
      relation = $transition.compareRoute(from, to);
    });
    _animationController.reset();
    _animationController.forward();
  }

  @override
  void dispose() {
    routerAnimationObserver.unsubscribe(this);
    _animationController.dispose();
    super.dispose();
  }

  Widget _applyAnimation(Widget child, RouteRelation relation) {
    const distance = 0.1;
    const curve = Curves.easeOutQuint;

    Animation<T> animate<T>(Tween<T> tween) {
      return _animationController.drive(tween.chain(CurveTween(curve: curve)));
    }

    Offset slideFrom = Offset.zero;
    double fadeFrom = 1;
    double scaleFrom = 1;

    switch (relation) {
      case RouteRelation.parent:
        slideFrom = const Offset(0, -distance);
        fadeFrom = 0;
      case RouteRelation.child:
        slideFrom = const Offset(0, distance);
        fadeFrom = 0;
      case RouteRelation.sameLevelAhead:
        slideFrom = const Offset(-distance, 0);
      case RouteRelation.sameLevelBehind:
        slideFrom = const Offset(distance, 0);
      case RouteRelation.same:
        break;
      case RouteRelation.crossLevel:
        scaleFrom = 1.1;
        fadeFrom = 0;
    }

    // The same widgets for every relation, so that a page revealed with
    // another relation than it was opened with keeps its state.
    return SlideTransition(
      position: animate(Tween<Offset>(begin: slideFrom, end: Offset.zero)),
      child: FadeTransition(
        opacity: animate(Tween<double>(begin: fadeFrom, end: 1)),
        child: ScaleTransition(
          scale: animate(Tween<double>(begin: scaleFrom, end: 1)),
          child: child,
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    FluentLocalizations.of(context);

    return _applyAnimation(widget.child, relation);
  }
}
