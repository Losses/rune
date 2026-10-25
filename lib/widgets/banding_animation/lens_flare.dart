import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter_shaders/flutter_shaders.dart';

class LensFlareEffect extends StatefulWidget {
  const LensFlareEffect({
    super.key,
    required this.flarePosition,
    required this.alpha,
    required this.child,
  });

  final Offset flarePosition;
  final double alpha;
  final Widget child;

  @override
  State<LensFlareEffect> createState() => _LensFlareEffectState();
}

class _LensFlareEffectState extends State<LensFlareEffect>
    with SingleTickerProviderStateMixin {
  late Future<ui.FragmentProgram> _shaderProgram;
  late final Ticker _ticker;
  double _time = 0.0;

  @override
  void initState() {
    super.initState();
    _shaderProgram = _loadShader();

    // Advance the shader time from the frame pipeline instead of a
    // free-running Timer.periodic. A Ticker follows the animation clock, is
    // muted by TickerMode together with the rest of the app, and stops as soon
    // as this widget is disposed.
    _ticker = createTicker(_handleTick)..start();
  }

  void _handleTick(Duration elapsed) {
    setState(() {
      // Keep the old speed: 0.1 unit per second of elapsed time.
      _time = elapsed.inMicroseconds / 10000000;
    });
  }

  Future<ui.FragmentProgram> _loadShader() async {
    try {
      return await ui.FragmentProgram.fromAsset('lib/shaders/lens_flare.frag');
    } catch (e) {
      rethrow;
    }
  }

  @override
  void dispose() {
    _ticker.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return FutureBuilder<ui.FragmentProgram>(
      future: _shaderProgram,
      builder: (context, snapshot) {
        if (!snapshot.hasData) {
          return Container();
        }

        final shader = snapshot.data!;

        return AnimatedSampler(
          (image, size, canvas) {
            final fragmentShader = shader.fragmentShader();

            fragmentShader
              ..setImageSampler(0, image)
              // resolution
              ..setFloat(0, size.width)
              ..setFloat(1, size.height)
              // u_alpha
              ..setFloat(2, widget.alpha)
              // u_time
              ..setFloat(3, _time)
              // u_mouse
              ..setFloat(4, widget.flarePosition.dx)
              ..setFloat(5, widget.flarePosition.dy);

            canvas.drawRect(
              Offset.zero & size,
              Paint()..shader = fragmentShader,
            );
          },
          child: widget.child,
        );
      },
    );
  }
}
