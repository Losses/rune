import 'package:fluent_ui/fluent_ui.dart';

enum AxRevealPressAnimationFillMode {
  normal,
  constrained,
}

class RevealConfig {
  final Color borderColor;
  final Color hoverLightColor;
  final Color pressAnimationColor;
  final double borderWidth;
  final double opacity;
  final double borderFillRadius;
  final double hoverLightFillRadius;
  final BorderRadiusGeometry borderRadius;
  final bool hoverLight;
  final bool diffuse;
  final bool pressAnimation;
  final AxRevealPressAnimationFillMode pressAnimationFillMode;

  const RevealConfig({
    this.borderColor = Colors.white,
    this.hoverLightColor = Colors.white,
    this.pressAnimationColor = Colors.white,
    this.borderWidth = 1.0,
    this.opacity = 0.26,
    this.borderFillRadius = 1.0,
    this.hoverLightFillRadius = 1.0,
    this.borderRadius = BorderRadius.zero,
    this.hoverLight = true,
    this.diffuse = true,
    this.pressAnimation = true,
    this.pressAnimationFillMode = AxRevealPressAnimationFillMode.normal,
  });
}
