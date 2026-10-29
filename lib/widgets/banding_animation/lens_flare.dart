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
  late Future<ui.FragmentShader> _shader;
  ui.FragmentShader? _fragmentShader;
  late final Ticker _ticker;
  double _time = 0.0;

  @override
  void initState() {
    super.initState();
    _shader = _loadShader();

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

  Future<ui.FragmentShader> _loadShader() async {
    try {
      final program = await ui.FragmentProgram.fromAsset(
        'lib/shaders/lens_flare.frag',
      );

      // Create the shader once and reuse it for every frame. A shader keeps
      // the image passed to setImageSampler alive until it is disposed, and
      // AnimatedSampler hands us a new full-size snapshot of the child every
      // frame. A fresh shader per frame left every snapshot to the garbage
      // collector; on iOS (Impeller) they piled up to gigabytes within a few
      // seconds and the app was killed for exceeding its memory limit.
      final shader = program.fragmentShader();

      // The widget can be removed before the asset finishes loading. Its
      // dispose has then already run with no shader to release.
      if (!mounted) {
        shader.dispose();
      } else {
        _fragmentShader = shader;
      }

      return shader;
    } catch (e) {
      rethrow;
    }
  }

  @override
  void dispose() {
    _ticker.dispose();
    _fragmentShader?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return FutureBuilder<ui.FragmentShader>(
      future: _shader,
      builder: (context, snapshot) {
        if (!snapshot.hasData) {
          return Container();
        }

        final fragmentShader = snapshot.data!;

        return AnimatedSampler(
          (image, size, canvas) {
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
