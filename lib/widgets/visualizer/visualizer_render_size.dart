import 'dart:math';
import 'dart:ui';

/// Physical texture pixels from local layout units and effective pixel density.
/// Root scaling shrinks layout and grows density by reciprocal amounts, so it
/// does not reduce the resolution of a surface occupying the same window area.
/// Keep the existing GPU budget and Windows readback row-alignment contract.
(int, int) visualizerRenderSize(Size logicalSize, double effectiveDpr) {
  if (!logicalSize.isFinite ||
      logicalSize.isEmpty ||
      !effectiveDpr.isFinite ||
      effectiveDpr <= 0) {
    throw ArgumentError(
      'Visualizer size and density must be finite and positive',
    );
  }
  final width = logicalSize.width * effectiveDpr;
  final height = logicalSize.height * effectiveDpr;
  final factor = min(1.0, min(1920 / width, 1080 / height));
  final renderWidth = max(64, (width * factor).floor() ~/ 64 * 64);
  // Extremely narrow windows cannot preserve aspect ratio and also satisfy
  // the 64-pixel row minimum. Keep height inside the renderer's budget.
  final renderHeight =
      (renderWidth * height / width).clamp(2, 1080).round() & ~1;
  return (renderWidth, renderHeight);
}
