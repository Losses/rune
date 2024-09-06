import 'dart:async';
import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_shaders/flutter_shaders.dart';

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
    this.mode = 1.0,
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
      return await ui.FragmentProgram.fromAsset('lib/shaders/gradient.frag');
    } catch (e) {
      rethrow;
    }
  }

  void _updateMousePosition(PointerEvent event) {
    final RenderBox renderBox =
        _containerKey.currentContext!.findRenderObject() as RenderBox;
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
          return widget.child;
        }

        final shader = snapshot.data!;

        return AnimatedSampler(
          (ui.Image image, Size size, Canvas canvas) {
            final fragmentShader = shader.fragmentShader();
            fragmentShader
              ..setImageSampler(0, image)
              // resolution
              ..setFloat(0, size.width)
              ..setFloat(1, size.height)
              // u_time
              ..setFloat(2, _time)
              // u_mouse
              ..setFloat(3, _mousePosition.dx)
              ..setFloat(4, _mousePosition.dy)
              // u_mode
              ..setFloat(5, widget.mode)
              // u_swap
              ..setFloat(6, widget.swap)
              // u_params
              ..setFloat(7, widget.gradientParams.multX)
              ..setFloat(8, widget.gradientParams.multY)
              ..setFloat(9, widget.gradientParams.hue)
              ..setFloat(10, widget.gradientParams.brightness)
              // u_params2
              ..setFloat(11, widget.effectParams.mouseInfluence)
              ..setFloat(12, widget.effectParams.scale)
              ..setFloat(13, widget.effectParams.noise)
              ..setFloat(14, widget.effectParams.bw)
              // u_altparams
              ..setFloat(15, widget.altParams.scale2)
              ..setFloat(16, widget.altParams.bw2)
              ..setFloat(17, 0.0) // Placeholder for future use
              ..setFloat(18, 0.0) // Placeholder for future use
              // u_color
              ..setFloat(19, widget.color.red / 255.0)
              ..setFloat(20, widget.color.green / 255.0)
              ..setFloat(21, widget.color.blue / 255.0)
              // u_color2
              ..setFloat(22, widget.color2.red / 255.0)
              ..setFloat(23, widget.color2.green / 255.0)
              ..setFloat(24, widget.color2.blue / 255.0);

            canvas.drawRect(
              Offset.zero & size,
              Paint()..shader = fragmentShader,
            );
          },
          key: _containerKey,
          child: widget.child,
        );
      },
    );
  }
}
