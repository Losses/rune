import 'dart:async';

import 'package:flutter/gestures.dart';
import 'package:fluent_ui/fluent_ui.dart';

import 'history.dart';
import 'navigation.dart';
import 'context_menu_route_entry.dart';

/// A flyout a [RouterAwareFlyoutController] has open: its entry in the
/// navigation history and the route it is shown on, which its own [settings]
/// tell from any other route.
class _OpenFlyout {
  _OpenFlyout(this._navigator, RouteSettings? settings) : _given = settings;

  /// The navigator the flyout was asked to be shown on, if any.
  final NavigatorState? _navigator;

  final RouteSettings? _given;

  late final entry = ContextMenuRouteEntry(name: 'flyout', pop: close);

  late final RouteSettings settings =
      _given ?? RouteSettings(name: 'flyout', arguments: entry);

  ModalRoute<dynamic>? _route;

  /// Whether the flyout was closed before its route could be found.
  bool _closeWhenShown = false;

  /// Closes the flyout's own route if it is still open: with its exit
  /// transition when it is the top route, and at once from under the routes
  /// above it otherwise. A route already on its way out, or gone, is left
  /// alone, so the route under a closing flyout is never popped in its place.
  void close() {
    final route = _route ??= _pushed();
    if (route == null) {
      _closeWhenShown = true;
      return;
    }
    if (!route.isActive) return;
    final navigator = route.navigator!;
    if (route.isCurrent) {
      navigator.pop();
    } else {
      navigator.removeRoute(route);
    }
  }

  /// The flyout's route while it is pushed and not yet built, when nothing
  /// was pushed over it since. A navigator tells its top route only to
  /// [NavigatorState.popUntil], which pops nothing when it accepts the first
  /// route it is asked about.
  ModalRoute<dynamic>? _pushed() {
    final asked = _navigator;
    final navigators = [
      if (asked != null) asked,
      if ($hasLibraryNavigator) $state(),
      if ($$hasRootNavigator) $$state(),
    ];
    for (final navigator in navigators) {
      ModalRoute<dynamic>? top;
      navigator.popUntil((route) {
        if (route is ModalRoute && identical(route.settings, settings)) {
          top = route;
        }
        return true;
      });
      if (top != null) return top;
    }
    return null;
  }

  void _shown(ModalRoute<dynamic> route) {
    if (_route != null) return;
    _route = route;
    // The navigator does not change while it builds.
    if (_closeWhenShown) {
      WidgetsBinding.instance.addPostFrameCallback((_) => close());
    }
  }
}

/// Tells [onRoute] the route its flyout is shown on. Only this widget depends
/// on the route, so the flyout is not built again when the route changes.
class _RouteProbe extends StatefulWidget {
  const _RouteProbe({required this.onRoute, required this.child});

  final void Function(ModalRoute<dynamic> route) onRoute;
  final Widget child;

  @override
  State<_RouteProbe> createState() => _RouteProbeState();
}

class _RouteProbeState extends State<_RouteProbe> {
  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final route = ModalRoute.of(context);
    if (route != null) widget.onRoute(route);
  }

  @override
  Widget build(BuildContext context) => widget.child;
}

/// A wrapper around [FlyoutController] that integrates with the application's
/// routing system to properly handle back navigation when flyouts are open.
class RouterAwareFlyoutController {
  /// The underlying flyout controller
  final FlyoutController _controller = FlyoutController();

  /// The flyouts shown and not yet closed, each with its own entry and route.
  final Set<_OpenFlyout> _flyouts = {};

  bool _disposed = false;

  /// Whether the owner has let go of this controller.
  bool get isDisposed => _disposed;

  /// Whether the flyout is currently open
  bool get isOpen => _controller.isOpen;

  /// Whether the controller is attached to a target
  bool get isAttached => _controller.isAttached;

  /// Register this controller with the navigation history when a flyout is opened.
  ///
  /// Menus load their data first and open afterwards, and the widget that owns
  /// the controller can be gone by then. Nothing is shown in that case.
  ///
  /// The flyout's entry in the history closes the flyout's own route, and
  /// leaves the history when the flyout is closed, however that happens.
  Future<T?> showFlyout<T>({
    required WidgetBuilder builder,
    bool barrierDismissible = true,
    bool dismissWithEsc = true,
    bool dismissOnPointerMoveAway = false,
    FlyoutPlacementMode placementMode = FlyoutPlacementMode.auto,
    FlyoutAutoConfiguration? autoModeConfiguration,
    bool forceAvailableSpace = false,
    bool shouldConstrainToRootBounds = true,
    double additionalOffset = 8.0,
    double margin = 8.0,
    Color? barrierColor,
    NavigatorState? navigatorKey,
    FlyoutTransitionBuilder? transitionBuilder,
    Duration? transitionDuration,
    Offset? position,
    RouteSettings? settings,
    GestureRecognizer? barrierRecognizer,
  }) async {
    if (_disposed) return null;

    // The entry is registered before the flyout is shown, so that a page
    // pushed meanwhile closes it.
    final flyout = _OpenFlyout(navigatorKey, settings);
    _flyouts.add(flyout);
    $history.pushContextMenu(flyout.entry);

    try {
      return await _controller.showFlyout<T>(
        builder: (context) {
          if (_disposed) {
            // The owner can disappear before the flyout's first build.
            scheduleMicrotask(flyout.close);
            return const SizedBox.shrink();
          }
          return _RouteProbe(onRoute: flyout._shown, child: builder(context));
        },
        barrierDismissible: barrierDismissible,
        dismissWithEsc: dismissWithEsc,
        dismissOnPointerMoveAway: dismissOnPointerMoveAway,
        placementMode: placementMode,
        autoModeConfiguration: autoModeConfiguration,
        forceAvailableSpace: forceAvailableSpace,
        shouldConstrainToRootBounds: shouldConstrainToRootBounds,
        additionalOffset: additionalOffset,
        margin: margin,
        barrierColor: barrierColor,
        navigatorKey: navigatorKey,
        transitionBuilder: transitionBuilder,
        transitionDuration: transitionDuration,
        position: position,
        settings: flyout.settings,
        barrierRecognizer: barrierRecognizer,
      );
    } finally {
      // The flyout is closed. A menu item may already have pushed a page on
      // top of its entry, or closed the entry along with its route.
      _flyouts.remove(flyout);
      $history.remove(flyout.entry);
      // Fluent notifies listeners after the Navigator completes its push.
      if (_disposed && _flyouts.isEmpty) _controller.dispose();
    }
  }

  /// Access to the underlying controller for other operations
  FlyoutController get controller => _controller;

  /// Closes the flyouts still open, each with its own entry wherever it is in
  /// the history, and lets go of the controller once they are closed.
  void dispose() {
    if (_disposed) return;
    _disposed = true;

    for (final flyout in _flyouts) {
      $history.remove(flyout.entry);
      // The owner is disposed while the tree is locked, so the navigator
      // closes the route after.
      scheduleMicrotask(flyout.close);
    }

    // FlyoutController extends ChangeNotifier, and a flyout still open tells
    // it when it closes.
    if (_flyouts.isEmpty) _controller.dispose();
  }
}
