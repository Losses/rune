import 'dart:async';
import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_shaders/flutter_shaders.dart';

class GradientParams {
  final double multX;
  final double multY;
  final double brightness;

  const GradientParams({
    this.multX = 2.0,
    this.multY = 2.0,
    this.brightness = 0.8,
  });
}

class EffectParams {
  final double mouseInfluence;
  final double scale;
  final double noise;
  final double bw;

  const EffectParams({
    this.mouseInfluence = -1.0,
    this.scale = 1.0,
    this.noise = 1.5,
    this.bw = 0.0,
  });
}

class GradientContainer extends StatefulWidget {
  final Widget child;
  final GradientParams gradientParams;
  final EffectParams effectParams;
  final Color color;
  final Color color2;

  const GradientContainer({
    required this.child,
    required this.gradientParams,
    required this.effectParams,
    required this.color,
    required this.color2,
    super.key,
  });

  @override
  GradientContainerState createState() => GradientContainerState();
}

class GradientContainerState extends State<GradientContainer> {
  late Future<(ui.FragmentProgram, FragmentShader)> _shaderProgram;
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

  Future<(ui.FragmentProgram, FragmentShader)> _loadShader() async {
    try {
      final fragmentProgram =
          await ui.FragmentProgram.fromAsset('lib/shaders/gradient.frag');
      final fragmentShader = fragmentProgram.fragmentShader();

      return (fragmentProgram, fragmentShader);
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
    return FutureBuilder<(ui.FragmentProgram, FragmentShader)>(
      future: _shaderProgram,
      builder: (context, snapshot) {
        if (!snapshot.hasData) {
          return widget.child;
        }

        final isDark = FluentTheme.of(context).brightness.isDark;

        final shader = snapshot.data!;
        final fragmentShader = shader.$2;

        return AnimatedSampler(
          (ui.Image image, Size size, Canvas canvas) {
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
              // u_params
              ..setFloat(5, widget.gradientParams.multX)
              ..setFloat(6, widget.gradientParams.multY)
              ..setFloat(7, 180.0)
              ..setFloat(
                8,
                isDark
                    ? widget.gradientParams.brightness
                    : widget.gradientParams.brightness * 2,
              )
              // u_params2
              ..setFloat(
                9,
                isDark
                    ? widget.effectParams.mouseInfluence
                    : widget.effectParams.mouseInfluence * -1,
              )
              ..setFloat(10, widget.effectParams.scale)
              ..setFloat(11, widget.effectParams.noise)
              ..setFloat(12, widget.effectParams.bw)
              // u_color
              ..setFloat(13, widget.color.r / 255.0)
              ..setFloat(14, widget.color.g / 255.0)
              ..setFloat(15, widget.color.b / 255.0)
              // u_color2
              ..setFloat(16, widget.color2.r / 255.0)
              ..setFloat(17, widget.color2.g / 255.0)
              ..setFloat(18, widget.color2.b / 255.0)
              // u_is_dark
              ..setFloat(19, isDark ? 1 : 0);

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
