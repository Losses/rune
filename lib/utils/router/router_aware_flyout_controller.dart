import 'dart:async';

import 'package:flutter/gestures.dart';
import 'package:fluent_ui/fluent_ui.dart';

import 'history.dart';
import 'navigation.dart';
import 'context_menu_route_entry.dart';

/// A wrapper around [FlyoutController] that integrates with the application's
/// routing system to properly handle back navigation when flyouts are open.
class RouterAwareFlyoutController {
  /// The underlying flyout controller
  final FlyoutController _controller = FlyoutController();

  /// Whether the flyout is currently open
  bool get isOpen => _controller.isOpen;

  /// Whether the controller is attached to a target
  bool get isAttached => _controller.isAttached;

  /// Register this controller with the navigation history when a flyout is opened
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
    // We need to register the modal entry before showing the flyout
    final modalEntry = ContextMenuRouteEntry(
      name: 'flyout',
      arguments: null,
      canPop: null,
      pop: () {
        if (_controller.isOpen) {
          // Since we can't directly access the context from the controller
          // we'll use the navigator that was passed or the current navigator
          final navigator = navigatorKey ?? Navigator.of($context());
          navigator.pop();
        }
      },
    );

    // Register this flyout with the navigation history system
    $history.pushContextMenu(modalEntry);

    try {
      // Show the actual flyout
      final result = await _controller.showFlyout<T>(
        builder: builder,
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

      // Clean up the navigation history entry when the flyout is closed
      if ($history.isCurrentContextMenu) {
        $history.pop();
      }

      // Return the result from the flyout
      return result;
    } catch (e) {
      // If there's an error, make sure we clean up the navigation history
      if ($history.isCurrentContextMenu) {
        $history.pop();
      }
      rethrow;
    }
  }

  /// Access to the underlying controller for other operations
  FlyoutController get controller => _controller;

  /// Disposes of resources used by this controller
  void dispose() {
    // Close any open flyout
    if (_controller.isOpen) {
      try {
        // Use the app's navigator to close the flyout
        Navigator.of($context()).pop();
      } catch (_) {
        // Ignore errors that might occur if the context is no longer valid
      }

      // Clean up any remaining modal entries for this flyout
      if ($history.isCurrentContextMenu) {
        $history.pop();
      }
    }

    // Dispose of the underlying controller
    // FlyoutController extends ChangeNotifier, so it needs to be disposed
    _controller.dispose();
  }
}
