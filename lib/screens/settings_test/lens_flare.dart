import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter_shaders/flutter_shaders.dart';

class LensFlareEffect extends StatefulWidget {
  const LensFlareEffect({super.key});

  @override
  State<LensFlareEffect> createState() => _LensFlareEffectState();
}

class _LensFlareEffectState extends State<LensFlareEffect> {
  late Future<ui.FragmentProgram> _shaderProgram;
  late Timer _timer;
  double _time = 0.0;
  Offset _mousePosition = Offset.zero;

  final GlobalKey _containerKey = GlobalKey();

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

    // Listen to mouse position updates
    WidgetsBinding.instance.pointerRouter.addGlobalRoute(_updateMousePosition);
  }

  Future<ui.FragmentProgram> _loadShader() async {
    try {
      return await ui.FragmentProgram.fromAsset('lib/shaders/lens_flare.frag');
    } catch (e) {
      rethrow;
    }
  }

  void _updateMousePosition(PointerEvent event) {
    final currentContext = _containerKey.currentContext;

    if (currentContext == null) return;

    final RenderBox renderBox = currentContext.findRenderObject() as RenderBox;
    final Offset localPosition = renderBox.globalToLocal(event.position);
    setState(() {
      _mousePosition = localPosition;
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
          return Container();
        }

        final shader = snapshot.data!;

        return AnimatedSampler(
          (image, size, canvas) {
            final fragmentShader = shader.fragmentShader();

            fragmentShader
              ..setFloat(0, _time)
              ..setFloat(1, size.width)
              ..setFloat(2, size.height)
              ..setFloat(3, _mousePosition.dx)
              ..setFloat(4, _mousePosition.dy);

            canvas.drawRect(
              Offset.zero & size,
              Paint()..shader = fragmentShader,
            );
          },
          key: _containerKey,
          child: Container(),
        );
      },
    );
  }
}
