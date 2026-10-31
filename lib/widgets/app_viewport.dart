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
    final factor = context.select<AppScaleProvider, double>(
      (scale) => scale.scaleFactor,
    );
    final media = MediaQuery.of(context);
    return LayoutBuilder(
      builder: (context, constraints) {
        final size = constraints.biggest / factor;
        final scaledMedia = media
            .copyWith(
              size: size,
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
