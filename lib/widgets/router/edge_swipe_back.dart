import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:provider/provider.dart';

import '../navigation_bar/page_chrome.dart';
import '../../providers/responsive_providers.dart';
import '../../utils/kept_clear.dart';
import '../../utils/navigation/utils/navigation_backward.dart';

/// Narrowest strip at the left edge that starts the swipe, in logical pixels.
const edgeSwipeMinWidth = 20.0;

/// A rightward drag longer than this, or than a quarter of the width if that
/// is shorter, goes back when the finger lifts.
const edgeSwipeDistance = 60.0;

/// A rightward fling faster than this goes back whatever its length.
const edgeSwipeVelocity = 300.0;

// As the back button in the header does.
void _goBack() => navigateBackwardWithPop();

/// On iOS, a drag from the left edge to the right calls [onBack] when the
/// finger lifts. The page does not follow the finger. Other platforms get
/// [child] as it is.
class EdgeSwipeBack extends StatelessWidget {
  const EdgeSwipeBack({
    super.key,
    required this.child,
    this.onBack = _goBack,
    this.enabled = true,
  });

  final Widget child;
  final VoidCallback onBack;
  final bool enabled;

  @override
  Widget build(BuildContext context) {
    if (defaultTargetPlatform != TargetPlatform.iOS) return child;

    final keptClear = context.keptClear;
    final chrome = pageChromeOf(
      Provider.of<ResponsiveProvider>(context),
      Provider.of<ScreenSizeProvider>(context).screenSize,
    );
    // The strip starts below the header, so it never covers the back button
    // in the header's top left corner.
    final top = chrome.header + keptClear.top;

    // The stack stays when the strip is off, so [child] keeps its place in
    // the tree.
    return Stack(
      fit: StackFit.passthrough,
      children: [
        child,
        if (enabled)
          Positioned(
            left: 0,
            top: top,
            bottom: 0,
            width: max(keptClear.left, edgeSwipeMinWidth),
            child: _Strip(onBack: onBack),
          ),
      ],
    );
  }
}

class _Strip extends StatefulWidget {
  const _Strip({required this.onBack});

  final VoidCallback onBack;

  @override
  State<_Strip> createState() => _StripState();
}

class _StripState extends State<_Strip> {
  double _dx = 0;

  @override
  Widget build(BuildContext context) {
    final width = MediaQuery.sizeOf(context).width;
    final distance = min(edgeSwipeDistance, width / 4);
    final settings = MediaQuery.gestureSettingsOf(context);

    final drag = GestureRecognizerFactoryWithHandlers<_RightwardDrag>(
      () => _RightwardDrag(debugOwner: this),
      (instance) {
        instance
          ..gestureSettings = settings
          ..onStart = (_) {
            _dx = 0;
          }
          ..onUpdate = (details) {
            _dx += details.delta.dx;
          }
          ..onEnd = (details) {
            final velocity = details.primaryVelocity ?? 0;
            if (_dx > distance || velocity > edgeSwipeVelocity) {
              // A drag that turned back left does not count.
              if (_dx > 0) widget.onBack();
            }
          };
      },
    );

    return RawGestureDetector(
      // The page under the strip still gets the touch, so taps, vertical
      // scrolls and leftward drags that start here work. Only a drag to the
      // right goes back.
      behavior: HitTestBehavior.translucent,
      gestures: {_RightwardDrag: drag},
    );
  }
}

/// A horizontal drag that claims the pointer only once it has gone the slop to
/// the right. One that goes to the left is left to the page under it, such as
/// a horizontal list there.
class _RightwardDrag extends HorizontalDragGestureRecognizer {
  _RightwardDrag({super.debugOwner});

  @override
  bool hasSufficientGlobalDistanceToAccept(
    PointerDeviceKind pointerDeviceKind,
    double? deviceTouchSlop,
  ) {
    return globalDistanceMoved >
        computeHitSlop(pointerDeviceKind, gestureSettings);
  }
}
