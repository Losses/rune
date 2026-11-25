import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';

/// A custom gesture recognizer for handling mouse forward (4th) and backward (5th) buttons
class NavigationMouseKeyRecognizer extends BaseTapGestureRecognizer {
  /// Callback for forward button tap down event
  GestureTapDownCallback? onForwardTapDown;

  /// Callback for forward button tap up event
  GestureTapUpCallback? onForwardTapUp;

  /// Callback for forward button tap cancel event
  GestureTapCancelCallback? onForwardTapCancel;

  /// Callback for backward button tap down event
  GestureTapDownCallback? onBackwardTapDown;

  /// Callback for backward button tap up event
  GestureTapUpCallback? onBackwardTapUp;

  /// Callback for backward button tap cancel event
  GestureTapCancelCallback? onBackwardTapCancel;

  /// Check if the pointer event should be handled by this recognizer
  @override
  bool isPointerAllowed(PointerDownEvent event) {
    if (event.buttons == kForwardMouseButton ||
        event.buttons == kBackMouseButton) {
      return super.isPointerAllowed(event);
    }
    return false;
  }

  /// Handle tap down events
  @override
  void handleTapDown({required PointerDownEvent down}) {
    final TapDownDetails details = TapDownDetails(
      globalPosition: down.position,
      localPosition: down.localPosition,
      kind: down.kind,
    );

    if (down.buttons == kForwardMouseButton) {
      onForwardTapDown?.call(details);
    } else if (down.buttons == kBackMouseButton) {
      onBackwardTapDown?.call(details);
    }
  }

  /// Handle tap up events
  @override
  void handleTapUp(
      {required PointerDownEvent down, required PointerUpEvent up}) {
    final TapUpDetails details = TapUpDetails(
      kind: up.kind,
      globalPosition: up.position,
      localPosition: up.localPosition,
    );

    if (down.buttons == kForwardMouseButton) {
      onForwardTapUp?.call(details);
    } else if (down.buttons == kBackMouseButton) {
      onBackwardTapUp?.call(details);
    }
  }

  /// Handle tap cancel events
  @override
  void handleTapCancel({
    required PointerDownEvent down,
    PointerCancelEvent? cancel,
    required String reason,
  }) {
    if (down.buttons == kForwardMouseButton) {
      onForwardTapCancel?.call();
    } else if (down.buttons == kBackMouseButton) {
      onBackwardTapCancel?.call();
    }
  }
}

/// A widget that listens to mouse forward and backward button events
class NavigationMouseKeyListener extends StatelessWidget {
  /// The child widget to wrap
  final Widget child;

  /// Hit test behavior for gesture detection
  final HitTestBehavior? behavior;

  /// Callback for forward button tap down
  final GestureTapDownCallback? onForwardMouseButtonTapDown;

  /// Callback for backward button tap down
  final GestureTapDownCallback? onBackwardMouseButtonTapDown;

  /// Constructor
  const NavigationMouseKeyListener({
    super.key,
    required this.child,
    this.behavior,
    this.onForwardMouseButtonTapDown,
    this.onBackwardMouseButtonTapDown,
  });

  @override
  Widget build(BuildContext context) {
    // Create a gesture recognizer factory
    final Map<Type, GestureRecognizerFactory> gestures =
        <Type, GestureRecognizerFactory>{
      NavigationMouseKeyRecognizer:
          GestureRecognizerFactoryWithHandlers<NavigationMouseKeyRecognizer>(
        () => NavigationMouseKeyRecognizer(),
        (NavigationMouseKeyRecognizer instance) {
          instance
            ..onForwardTapDown = onForwardMouseButtonTapDown
            ..onBackwardTapDown = onBackwardMouseButtonTapDown;
        },
      ),
    };

    // Wrap the child with RawGestureDetector
    return RawGestureDetector(
      gestures: gestures,
      behavior: behavior ?? HitTestBehavior.translucent,
      child: child,
    );
  }
}
