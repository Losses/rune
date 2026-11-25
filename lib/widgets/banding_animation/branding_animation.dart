import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/asset_helper.dart';
import '../../utils/settings_manager.dart';
import '../../utils/api/sfx_play.dart';
import '../../constants/configurations.dart';

import 'branding_animation_implementation.dart';

class BrandingAnimation extends StatefulWidget {
  const BrandingAnimation({super.key, this.onCompleted});

  /// Called once the entrance animation has played to completion.
  ///
  /// The branding animation is a one-shot startup overlay; the host uses this
  /// to remove it from the widget tree instead of keeping an invisible layer
  /// that still schedules frames.
  final VoidCallback? onCompleted;

  @override
  State<BrandingAnimation> createState() => _BrandingAnimationState();
}

/// The startup sound until one is chosen. iOS starts without one, since the
/// sound would stop the audio of other apps every time Rune opens.
String get defaultStartUpSfx =>
    defaultTargetPlatform == TargetPlatform.iOS ? 'mute' : 'tech';

/// The asset of the startup sound that [config] names, or null for none.
String? startUpSfxAsset(String? config) {
  switch (config ?? defaultStartUpSfx) {
    case 'mute':
      return null;
    case 'fantasy':
      return 'assets/startup_0.ogg';
    default:
      return 'assets/startup_1.ogg';
  }
}

Future<String?> getStartUpSfxPath() async {
  final config = await SettingsManager().getValue<String?>(kBandingSfxKey);
  return startUpSfxAsset(config);
}

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
      final sfxPath = await getStartUpSfxPath();

      if (sfxPath != null) {
        sfxPlay(
          (await AssetHelper.instance.getAudioFileFromAssets(sfxPath)).path,
        );
      }
      if (!mounted) {
        return;
      }
      await _controller.forward().orCancel;
      if (mounted) {
        widget.onCompleted?.call();
      }
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
