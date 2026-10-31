import 'package:flutter/widgets.dart';

/// Invalidates popup geometry after viewport metrics have changed. Subclasses
/// capture the particular route or entry so a newer popup is never dismissed.
mixin PopupMetricsObserver<T extends StatefulWidget> on State<T> {
  (Size, double, EdgeInsets, EdgeInsets, EdgeInsets)? _popupMetrics;

  VoidCallback? capturePopupInvalidation();

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final media = MediaQuery.of(context);
    final metrics = (
      media.size,
      media.devicePixelRatio,
      media.padding,
      media.viewPadding,
      media.viewInsets
    );
    if (_popupMetrics != null && _popupMetrics != metrics) {
      final invalidate = capturePopupInvalidation();
      if (invalidate != null) {
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (mounted) invalidate();
        });
      }
    }
    _popupMetrics = metrics;
  }
}
