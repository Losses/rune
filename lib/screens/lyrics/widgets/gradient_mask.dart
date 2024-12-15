import 'package:flutter/material.dart';

class GradientMask extends StatelessWidget {
  final Widget child;
  final double percentage;

  const GradientMask({
    super.key,
    required this.child,
    this.percentage = 30,
  }) : assert(percentage >= 0 && percentage <= 50);

  @override
  Widget build(BuildContext context) {
    return ShaderMask(
      shaderCallback: (Rect bounds) {
        return LinearGradient(
          begin: Alignment.topCenter,
          end: Alignment.bottomCenter,
          stops: [
            0.0,
            percentage / 100,
            percentage / 100,
            (100 - percentage) / 100,
            (100 - percentage) / 100,
            1.0
          ],
          colors: const [
            Colors.transparent,
            Colors.white,
            Colors.white,
            Colors.white,
            Colors.white,
            Colors.transparent,
          ],
        ).createShader(bounds);
      },
      blendMode: BlendMode.dstIn,
      child: child,
    );
  }
}
