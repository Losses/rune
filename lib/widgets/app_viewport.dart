import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/widgets.dart';
import 'package:provider/provider.dart';

import '../providers/app_scale.dart';

/// A single coordinate space for the Navigator, its overlays and all app UI.
class AppViewport extends StatelessWidget {
  const AppViewport({super.key, required this.child});
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final (factor, following) = context
        .select<AppScaleProvider, (double, bool)>(
          (scale) => (scale.scaleFactor, scale.followingTextSize),
        );
    final media = MediaQuery.of(context);
    return LayoutBuilder(
      builder: (context, constraints) {
        final size = constraints.biggest / factor;
        final scaledMedia = media
            .copyWith(
              size: size,
              // A zoom that follows the text size stands in for it, and the
              // text gets what the zoom leaves. Where the zoom is 1, the text
              // is scaled as usual.
              textScaler: following && factor > 1
                  ? _BeyondZoomTextScaler(media.textScaler, factor)
                  : media.textScaler,
              devicePixelRatio: media.devicePixelRatio * factor,
              padding: media.padding / factor,
              viewPadding: media.viewPadding / factor,
              viewInsets: media.viewInsets / factor,
              systemGestureInsets: media.systemGestureInsets / factor,
              displayFeatures: media.displayFeatures
                  .map(
                    (feature) => ui.DisplayFeature(
                      bounds: Rect.fromLTRB(
                        feature.bounds.left / factor,
                        feature.bounds.top / factor,
                        feature.bounds.right / factor,
                        feature.bounds.bottom / factor,
                      ),
                      type: feature.type,
                      state: feature.state,
                    ),
                  )
                  .toList(),
            )
            .applyDisplayCornerRadii(
              media.displayCornerRadii == null
                  ? null
                  : media.displayCornerRadii! / factor,
            );
        return FittedBox(
          alignment: Alignment.topLeft,
          child: SizedBox.fromSize(
            size: size,
            child: MediaQuery(data: scaledMedia, child: child),
          ),
        );
      },
    );
  }
}

/// What is left of the [system] text size once the interface is zoomed by
/// [zoom]: text ends up as large as the system makes it, or as large as the
/// zoom makes it if that is larger. A zoom held at the layout floor leaves
/// the rest of a large text size to this; one rounded up to whole pixels
/// does not make the text smaller than the layout around it.
@immutable
class _BeyondZoomTextScaler extends TextScaler {
  const _BeyondZoomTextScaler(this.system, this.zoom);

  final TextScaler system;
  final double zoom;

  @override
  double scale(double fontSize) =>
      math.max(fontSize, system.scale(fontSize) / zoom);

  @override
  // Required by TextScaler, which deprecates it.
  // ignore: deprecated_member_use
  double get textScaleFactor => math.max(1.0, system.textScaleFactor / zoom);

  @override
  bool operator ==(Object other) =>
      other is _BeyondZoomTextScaler &&
      other.system == system &&
      other.zoom == zoom;

  @override
  int get hashCode => Object.hash(system, zoom);

  @override
  String toString() => 'beyond a zoom of ${zoom}x: $system';
}
