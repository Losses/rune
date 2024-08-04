import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_shaders/flutter_shaders.dart';
import 'dart:ui' as ui;

class GradientParams {
  final double multX;
  final double multY;
  final double hue;
  final double brightness;

  GradientParams({
    this.multX = 2.0,
    this.multY = 2.0,
    this.hue = 180.0,
    this.brightness = 0.8,
  });
}

class EffectParams {
  final double mouseInfluence;
  final double scale;
  final double noise;
  final double bw;

  EffectParams({
    this.mouseInfluence = -1.0,
    this.scale = 1.0,
    this.noise = 1.5,
    this.bw = 0.0,
  });
}

class AltParams {
  final double scale2;
  final double bw2;

  const AltParams({
    this.scale2 = 1.0,
    this.bw2 = 0.0,
  });
}

const defaultAltParameter = AltParams(
  scale2: 1.0,
  bw2: 0.0,
);

class GradientContainer extends StatefulWidget {
  final Widget child;
  final double mode;
  final double swap;
  final GradientParams gradientParams;
  final EffectParams effectParams;
  final AltParams altParams;
  final Color color;
  final Color color2;

  const GradientContainer({
    required this.child,
    this.mode = 0.0,
    this.swap = 0.0,
    required this.gradientParams,
    required this.effectParams,
    this.altParams = defaultAltParameter,
    required this.color,
    this.color2 = Colors.white,
    super.key,
  });

  @override
  GradientContainerState createState() => GradientContainerState();
}

class GradientContainerState extends State<GradientContainer> {
  late Future<ui.FragmentProgram> _shaderProgram;
  late Timer _timer;
  double _time = 0.0;
  Offset _mousePosition = Offset.zero;

  @override
  void initState() {
    super.initState();
    _shaderProgram = _loadShader();

    // Start a timer to update the time every frame
    _timer = Timer.periodic(const Duration(milliseconds: 16), (Timer timer) {
      setState(() {
        _time += 0.016; // Increment time by 16 milliseconds
      });
    });

    // Listen to mouse position updates
    WidgetsBinding.instance.pointerRouter.addGlobalRoute(_updateMousePosition);
  }

  Future<ui.FragmentProgram> _loadShader() async {
    return ui.FragmentProgram.fromAsset('shaders/black_white_gradient.frag');
  }

  void _updateMousePosition(PointerEvent event) {
    setState(() {
      _mousePosition = event.localPosition;
    });
  }

  @override
  void dispose() {
    _timer.cancel();
    WidgetsBinding.instance.pointerRouter
        .removeGlobalRoute(_updateMousePosition);
    super.dispose();
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
              ..setFloat(2, _time)
              ..setFloat(3, widget.mode)
              ..setFloat(4, widget.swap)
              ..setFloat(5, widget.gradientParams.multX)
              ..setFloat(6, widget.gradientParams.multY)
              ..setFloat(7, widget.gradientParams.hue)
              ..setFloat(8, widget.gradientParams.brightness)
              ..setFloat(9, _mousePosition.dx)
              ..setFloat(10, _mousePosition.dy)
              ..setFloat(11, widget.effectParams.scale)
              ..setFloat(12, widget.effectParams.noise)
              ..setFloat(13, widget.effectParams.bw)
              ..setFloat(14, widget.altParams.scale2)
              ..setFloat(15, widget.altParams.bw2)
              ..setFloat(16, 0.0) // Placeholder for future use
              ..setFloat(17, 0.0) // Placeholder for future use
              ..setFloat(18, widget.color.red / 255.0)
              ..setFloat(19, widget.color.green / 255.0)
              ..setFloat(20, widget.color.blue / 255.0)
              ..setFloat(21, widget.color2.red / 255.0)
              ..setFloat(22, widget.color2.green / 255.0)
              ..setFloat(23, widget.color2.blue / 255.0);

            canvas.drawRect(
                Offset.zero & size, Paint()..shader = fragmentShader);
          },
          child: widget.child,
        );
      },
    );
  }
}
