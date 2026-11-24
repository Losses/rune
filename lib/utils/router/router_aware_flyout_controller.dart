import 'dart:async';

import 'package:flutter/gestures.dart';
import 'package:fluent_ui/fluent_ui.dart';

import 'history.dart';
import 'context_menu_route_entry.dart';

/// Hands each flyout of a [RouterAwareFlyoutController] the route it is shown
/// on as the route is pushed, so the flyout can close its own route before it
/// is built and after other routes are pushed over it.
///
/// Every navigator that shows these flyouts needs one. An observer watches one
/// navigator at a time, so each navigator has its own.
class FlyoutRouteObserver extends NavigatorObserver {
  @override
  void didPush(Route<dynamic> route, Route<dynamic>? previousRoute) {
    final settings = route.settings;
    if (settings is _FlyoutSettings && route is ModalRoute) {
      settings.flyout._route = route;
    }
  }
}

/// The settings a flyout's route is pushed with. They are the flyout's own, so
/// they tell [FlyoutRouteObserver] which flyout the route belongs to.
class _FlyoutSettings extends RouteSettings {
  _FlyoutSettings(this.flyout, RouteSettings settings)
    : super(name: settings.name, arguments: settings.arguments);

  final _OpenFlyout flyout;
}

/// A flyout a [RouterAwareFlyoutController] has open: its entry in the
/// navigation history and the route it is shown on.
class _OpenFlyout {
  _OpenFlyout(this._given);

  final RouteSettings? _given;

  late final entry = ContextMenuRouteEntry(name: 'flyout', pop: close);

  late final RouteSettings settings = _FlyoutSettings(
    this,
    _given ?? RouteSettings(name: 'flyout', arguments: entry),
  );

  /// The route the flyout is shown on, from the moment it is pushed. A flyout
  /// shown on a navigator without a [FlyoutRouteObserver] has none.
  ModalRoute<dynamic>? _route;

  /// Closes the flyout's own route if it is still open: with its exit
  /// transition when it is the top route, and at once from under the routes
  /// above it otherwise. A route already on its way out, or gone, is left
  /// alone, so the route under a closing flyout is never popped in its place.
  void close() {
    final route = _route;
    if (route == null || !route.isActive) return;
    final navigator = route.navigator!;
    if (route.isCurrent) {
      navigator.pop();
    } else {
      navigator.removeRoute(route);
    }
  }
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
  /// leaves the history when the flyout is closed, however that happens. The
  /// route comes from the [FlyoutRouteObserver] of the navigator the flyout is
  /// shown on.
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
    final flyout = _OpenFlyout(settings);
    _flyouts.add(flyout);
    $history.pushContextMenu(flyout.entry);

    try {
      return await _controller.showFlyout<T>(
        builder: (context) {
          // The owner can disappear before the flyout's first build, and its
          // dispose has already asked for the route to be closed.
          if (_disposed) return const SizedBox.shrink();
          return builder(context);
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
