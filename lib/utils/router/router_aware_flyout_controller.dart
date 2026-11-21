import 'dart:async';

import 'package:flutter/gestures.dart';
import 'package:fluent_ui/fluent_ui.dart';

import 'history.dart';
import 'context_menu_route_entry.dart';

/// A wrapper around [FlyoutController] that integrates with the application's
/// routing system to properly handle back navigation when flyouts are open.
class RouterAwareFlyoutController {
  /// The underlying flyout controller
  final FlyoutController _controller = FlyoutController();

  bool _disposed = false;
  final _openFlyouts = <VoidCallback>{};

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

    ModalRoute<dynamic>? route;
    void removeFlyout() {
      final current = route;
      if (current != null && current.isActive) {
        current.navigator?.removeRoute(current);
      }
    }

    final modalEntry = ContextMenuRouteEntry(
      name: 'flyout',
      arguments: null,
      canPop: null,
      pop: () {
        final current = route;
        if (current != null && current.isCurrent) {
          current.navigator?.pop();
        }
      },
    );

    _openFlyouts.add(removeFlyout);
    $history.pushContextMenu(modalEntry);

    try {
      return await _controller.showFlyout<T>(
        builder: (context) {
          route = ModalRoute.of(context);
          if (_disposed) {
            // The owner can disappear before the flyout's first build.
            scheduleMicrotask(removeFlyout);
            return const SizedBox.shrink();
          }
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
        settings: settings,
        barrierRecognizer: barrierRecognizer,
      );
    } finally {
      $history.remove(modalEntry);
      _openFlyouts.remove(removeFlyout);
      // Fluent notifies listeners after the Navigator completes its push.
      if (_disposed && _openFlyouts.isEmpty) _controller.dispose();
    }
  }

  /// Access to the underlying controller for other operations
  FlyoutController get controller => _controller;

  /// Disposes of resources used by this controller
  void dispose() {
    if (_disposed) return;
    _disposed = true;
    if (_openFlyouts.isEmpty) {
      _controller.dispose();
      return;
    }
    // Unmount runs while the widget tree is locked. Remove each owned route
    // after this frame's synchronous work, even if another route covers it.
    for (final remove in _openFlyouts) {
      scheduleMicrotask(remove);
    }
  }
}
