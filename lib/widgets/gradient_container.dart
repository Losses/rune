import 'package:flutter/material.dart';
import 'package:flutter_shaders/flutter_shaders.dart';
import 'dart:ui' as ui;

class GradientContainer extends StatefulWidget {
  final Widget child;
  final ui.Image gradientImage;

  const GradientContainer(
      {required this.child, required this.gradientImage, super.key});

  @override
  GradientContainerState createState() => GradientContainerState();
}

class GradientContainerState extends State<GradientContainer> {
  late Future<ui.FragmentProgram> _shaderProgram;

  @override
  void initState() {
    super.initState();
    _shaderProgram = _loadShader();
  }

  Future<ui.FragmentProgram> _loadShader() async {
    return ui.FragmentProgram.fromAsset('shaders/gradient.frag');
  }

  @override
  Widget build(BuildContext context) {
    return FutureBuilder<ui.FragmentProgram>(
      future: _shaderProgram,
      builder: (context, snapshot) {
        if (!snapshot.hasData) {
          return widget.child;
        }

        final shader = snapshot.data!;

        return AnimatedSampler(
          (ui.Image image, Size size, Canvas canvas) {
            final fragmentShader = shader.fragmentShader();
            fragmentShader
              ..setFloat(0, size.width)
              ..setFloat(1, size.height)
              ..setImageSampler(0, image)
              ..setImageSampler(1, widget.gradientImage);

            canvas.drawRect(
                Offset.zero & size, Paint()..shader = fragmentShader);
          },
          child: widget.child,
        );
      },
    );
  }
}
