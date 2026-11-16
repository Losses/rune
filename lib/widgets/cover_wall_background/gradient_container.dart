import 'dart:math';
import 'dart:ui' as ui;

import 'package:flutter/scheduler.dart';
import 'package:fluent_ui/fluent_ui.dart';

import 'gradient_noise.dart';
import 'shader_sampler.dart';

// The final hue shift of the gradient, in degrees.
const double _hue = 180.0;

// The shader's 2 pi, for the sway of the hue.
const double _shaderTau = 6.28318530718;

/// Sets the uniforms of lib/shaders/gradient.frag for one drawing of [image],
/// [size] large, at [time], with the pointer at [mouse] in the drawing's
/// coordinates. [lattice] keeps the noise gradients between drawings.
void setGradientUniforms(
  ui.FragmentShader shader, {
  required ui.Image image,
  required Size size,
  required double time,
  required Offset mouse,
  required GradientParams gradientParams,
  required EffectParams effectParams,
  required Color color,
  required Color color2,
  required bool isDark,
  required NoiseLattice lattice,
}) {
  shader
    ..setImageSampler(0, image)
    // resolution
    ..setFloat(0, size.width)
    ..setFloat(1, size.height)
    // u_time
    ..setFloat(2, time)
    // u_mouse
    ..setFloat(3, mouse.dx)
    ..setFloat(4, mouse.dy)
    // u_params
    ..setFloat(5, gradientParams.multX)
    ..setFloat(6, gradientParams.multY)
    ..setFloat(7, _hue)
    ..setFloat(
      8,
      isDark ? gradientParams.brightness : gradientParams.brightness * 2,
    )
    // u_params2
    ..setFloat(
      9,
      isDark ? effectParams.mouseInfluence : effectParams.mouseInfluence * -1,
    )
    ..setFloat(10, effectParams.scale)
    ..setFloat(11, effectParams.noise)
    ..setFloat(12, effectParams.bw)
    // u_color
    ..setFloat(13, color.r)
    ..setFloat(14, color.g)
    ..setFloat(15, color.b)
    // u_color2
    ..setFloat(16, color2.r)
    ..setFloat(17, color2.g)
    ..setFloat(18, color2.b)
    // u_is_dark
    ..setFloat(19, isDark ? 1 : 0);

  // What the shader no longer computes for each pixel: the cosines and sines
  // of the hue shifts, and the gradients of the noise.
  final shaderTime = asFloat32(time);
  final sway = sin(shaderTime) * _shaderTau / 30;
  final turn = _hue * pi / 180;
  shader
    // u_hue_sway
    ..setFloat(20, cos(sway))
    ..setFloat(21, sin(sway))
    // u_hue_turn
    ..setFloat(22, cos(turn))
    ..setFloat(23, sin(turn));
  // u_lattice0_mm to u_lattice1_pp
  final gradients = lattice.at(shaderTime);
  for (var i = 0; i < gradients.length; i++) {
    shader.setFloat(24 + i, gradients[i]);
  }
}

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

  final _lattice = NoiseLattice();

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
        // The noise lattice the app computes covers a scale below 2 (see
        // NoiseLattice).
        assert(widget.effectParams.scale < 2);

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
            setGradientUniforms(
              fragmentShader,
              image: image,
              size: size,
              time: _time.value,
              mouse: mousePosition,
              gradientParams: widget.gradientParams,
              effectParams: widget.effectParams,
              color: widget.color,
              color2: widget.color2,
              isDark: isDark,
              lattice: _lattice,
            );

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
