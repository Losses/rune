import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
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

class _LensFlareEffectState extends State<LensFlareEffect> {
  late Future<ui.FragmentProgram> _shaderProgram;
  late Timer _timer;
  double _time = 0.0;

  @override
  void initState() {
    super.initState();
    _shaderProgram = _loadShader();

    // Start a timer to update the time every frame
    _timer = Timer.periodic(const Duration(milliseconds: 16), (Timer timer) {
      setState(() {
        _time += 0.016 / 10; // Increment time by 16 milliseconds
      });
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
    _timer.cancel();
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
