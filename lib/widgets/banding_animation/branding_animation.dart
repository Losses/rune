import 'dart:io';
import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/asset_helper.dart';
import '../../utils/api/sfx_play.dart';

import 'branding_animation_implementation.dart';

class BrandingAnimation extends StatefulWidget {
  const BrandingAnimation({super.key});

  @override
  State<BrandingAnimation> createState() => _BrandingAnimationState();
}

Future<File> startSfxFile =
    AssetHelper.instance.getAudioFileFromAssets('assets/startup_1.ogg');

class _BrandingAnimationState extends State<BrandingAnimation>
    with TickerProviderStateMixin {
  late AnimationController _controller;

  // Define the size of the safe area
  static const double safeAreaSize = 400.0;
  static const double marginRatio = 0.12; // 12% margin ratio
  static const double designWidth = 1280.0;
  static const double designHeight = 720.0;

  @override
  void initState() {
    super.initState();

    _controller = AnimationController(
      duration: const Duration(milliseconds: 4500),
      vsync: this,
    );
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _playAnimation();
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  Future<void> _playAnimation() async {
    try {
      _controller.reset();
      sfxPlay((await startSfxFile).path);
      await _controller.forward().orCancel;
    } on TickerCanceled {
      // The animation got canceled, probably because we were disposed.
    }
  }

  double _calculateOptimalScale(BoxConstraints constraints) {
    final double screenWidth = constraints.maxWidth;
    final double screenHeight = constraints.maxHeight;

    // Calculate margin
    final double margin = min(screenWidth, screenHeight) * marginRatio;

    // Minimum size required for the safe area considering margins
    final double minRequiredWidth = safeAreaSize + (margin * 2);
    final double minRequiredHeight = safeAreaSize + (margin * 2);

    // Calculate the minimum scale based on the safe area
    final double safeScaleX = screenWidth / minRequiredWidth;
    final double safeScaleY = screenHeight / minRequiredHeight;
    final double safeScale = min(safeScaleX, safeScaleY);

    // Calculate the scale based on the original design dimensions
    final double designScaleX = screenWidth / designWidth;
    final double designScaleY = screenHeight / designHeight;
    final double designScale = min(designScaleX, designScaleY);

    // Return the larger of the two scales to ensure the safe area is not too small
    return min(1, max(safeScale, designScale));
  }

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final scale = _calculateOptimalScale(constraints);

        return Center(
          child: Transform.scale(
            scale: scale,
            child: OverflowBox(
              maxWidth: designWidth,
              maxHeight: designHeight,
              child: Center(
                child: BrandingAnimationImplementation(
                    controller: _controller.view),
              ),
            ),
          ),
        );
      },
    );
  }
}
