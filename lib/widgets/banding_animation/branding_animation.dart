import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/widgets/banding_animation/branding_animation_implementation.dart';

import '../../utils/api/sfx_play.dart';
import '../../screens/settings_test/settings_test.dart';

class BrandingAnimation extends StatefulWidget {
  const BrandingAnimation({super.key});

  @override
  State<BrandingAnimation> createState() => _BrandingAnimationState();
}

class _BrandingAnimationState extends State<BrandingAnimation>
    with TickerProviderStateMixin {
  late AnimationController _controller;

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

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final scaleX = constraints.maxWidth / 1280;
        final scaleY = constraints.maxHeight / 720;
        final scale = scaleX < scaleY ? scaleX : scaleY;

        return Center(
          child: Transform.scale(
            scale: scale,
            child: OverflowBox(
              maxWidth: 1280,
              maxHeight: 720,
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
