import 'reveal_effect_controller.dart';

class AnimationUnit {
  int? currentFrameId;
  int? mouseDownAnimateStartFrame;
  int mouseDownAnimateCurrentFrame = 0;
  double mouseDownAnimateLogicFrame = 0;
  int? mouseDownAnimateReleasedFrame;
  bool mouseReleased = false;
  bool cleanedUpForAnimation = false;
  RevealEffectController controller;

  final double pressAnimationSpeed;
  final double releaseAnimationAccelerateRate;

  AnimationUnit({
    required this.controller,
    this.pressAnimationSpeed = 60.0,
    this.releaseAnimationAccelerateRate = 2.0,
  });

  void reset() {
    currentFrameId = null;
    mouseDownAnimateStartFrame = null;
    mouseDownAnimateCurrentFrame = 0;
    mouseDownAnimateLogicFrame = 0;
    mouseDownAnimateReleasedFrame = null;
    mouseReleased = false;
    cleanedUpForAnimation = false;
  }
}
