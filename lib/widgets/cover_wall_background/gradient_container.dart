import 'dart:ui' as ui;

import 'package:flutter/scheduler.dart';
import 'package:fluent_ui/fluent_ui.dart';

import 'shader_sampler.dart';

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

class GradientContainerState extends State<GradientContainer>
    with SingleTickerProviderStateMixin {
  late final Future<(ui.FragmentProgram, ui.FragmentShader)> _shaderProgram;

  // The gradient moves every frame, and the pointer moves it too. Both reach
  // the shader through these notifiers, which draw the shader layer again
  // without a rebuild (see ShaderSampler). The ticker follows the frame clock
  // and stops while the page is hidden, for example under another page.
  late final Ticker _ticker = createTicker(_onTick);
  final ValueNotifier<double> _time = ValueNotifier(0.0);
  final ValueNotifier<Offset?> _globalMousePosition = ValueNotifier(null);
  late final Listenable _shaderInputs = Listenable.merge([
    _time,
    _globalMousePosition,
  ]);

  final GlobalKey _containerKey = GlobalKey();

  @override
  void initState() {
    super.initState();
    _shaderProgram = _loadShader();
    _shaderProgram.then((_) {
      if (mounted) _ticker.start();
    }, onError: (_) {});

    // Listen to mouse position updates
    WidgetsBinding.instance.pointerRouter.addGlobalRoute(_updateMousePosition);
  }

  void _onTick(Duration elapsed) {
    // A tenth of a unit each second.
    _time.value = elapsed.inMicroseconds / Duration.microsecondsPerSecond / 10;
  }

  Future<(ui.FragmentProgram, ui.FragmentShader)> _loadShader() async {
    try {
      final fragmentProgram = await ui.FragmentProgram.fromAsset(
        'lib/shaders/gradient.frag',
      );
      final fragmentShader = fragmentProgram.fragmentShader();

      return (fragmentProgram, fragmentShader);
    } catch (e) {
      rethrow;
    }
  }

  void _updateMousePosition(PointerEvent event) {
    _globalMousePosition.value = event.position;
  }

  @override
  void dispose() {
    _ticker.dispose();
    WidgetsBinding.instance.pointerRouter.removeGlobalRoute(
      _updateMousePosition,
    );
    _time.dispose();
    _globalMousePosition.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return FutureBuilder<(ui.FragmentProgram, ui.FragmentShader)>(
      future: _shaderProgram,
      builder: (context, snapshot) {
        if (!snapshot.hasData) {
          return widget.child;
        }

        final isDark = FluentTheme.of(context).brightness.isDark;

        final shader = snapshot.data!;
        final fragmentShader = shader.$2;

        return ShaderSampler(
          (ui.Image image, Size size, Canvas canvas) {
            final renderBox =
                _containerKey.currentContext?.findRenderObject() as RenderBox?;
            final globalMousePosition = _globalMousePosition.value;
            final mousePosition =
                globalMousePosition != null && renderBox != null
                ? renderBox.globalToLocal(globalMousePosition)
                : Offset.zero;
            fragmentShader
              ..setImageSampler(0, image)
              // resolution
              ..setFloat(0, size.width)
              ..setFloat(1, size.height)
              // u_time
              ..setFloat(2, _time.value)
              // u_mouse
              ..setFloat(3, mousePosition.dx)
              ..setFloat(4, mousePosition.dy)
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
              ..setFloat(13, widget.color.r)
              ..setFloat(14, widget.color.g)
              ..setFloat(15, widget.color.b)
              // u_color2
              ..setFloat(16, widget.color2.r)
              ..setFloat(17, widget.color2.g)
              ..setFloat(18, widget.color2.b)
              // u_is_dark
              ..setFloat(19, isDark ? 1 : 0);

            canvas.drawRect(
              Offset.zero & size,
              Paint()..shader = fragmentShader,
            );
          },
          repaint: _shaderInputs,
          key: _containerKey,
          child: widget.child,
        );
      },
    );
  }
}
