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

/// Sets the uniforms of lib/shaders/gradient_field.frag for one drawing
/// [size] large, at [time], with the pointer at [mouse] in the drawing's
/// coordinates. [lattice] keeps the noise gradients between drawings.
void setGradientUniforms(
  ui.FragmentShader shader, {
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

/// Draws the cover wall: the covers in grey, tinted by a field of slowly
/// moving colour.
///
/// The colour field (lib/shaders/gradient_field.frag) is laid out in the
/// wall's own coordinates and changes over a large part of it, whatever its
/// size. So it is worked out on a grid at most [fieldSize] pixels along the
/// wall's longer side, into an image that lib/shaders/gradient.frag scales
/// back up and lays over the covers, which keep their full resolution. On a
/// grid of 320 no pixel moves by more than a level of 255, from 21:9 to a phone
/// held upright (lib/test/widgets/gradient_shader_test.dart), so every platform
/// draws it so: the GPU works the field out for at most 321 by 321 pixels
/// instead of every pixel of the wall.
class CoverWallGradient {
  CoverWallGradient(this._field, this._composite, {this.fieldSize = 320});

  static Future<CoverWallGradient> load() async {
    final (field, composite) = await (
      ui.FragmentProgram.fromAsset('lib/shaders/gradient_field.frag'),
      ui.FragmentProgram.fromAsset('lib/shaders/gradient.frag'),
    ).wait;
    return CoverWallGradient(
      field.fragmentShader(),
      composite.fragmentShader(),
    );
  }

  final ui.FragmentShader _field;
  final ui.FragmentShader _composite;
  final int fieldSize;

  /// The field the last frame drew, kept until the next one replaces it.
  ui.Image? _lastField;

  /// Draws [covers] over [size] at [time], with the pointer at [mouse].
  void paint(
    Canvas canvas,
    Size size,
    ui.Image covers, {
    required double time,
    required Offset mouse,
    required GradientParams gradientParams,
    required EffectParams effectParams,
    required Color color,
    required Color color2,
    required bool isDark,
    required NoiseLattice lattice,
  }) {
    setGradientUniforms(
      _field,
      size: size,
      time: time,
      mouse: mouse,
      gradientParams: gradientParams,
      effectParams: effectParams,
      color: color,
      color2: color2,
      isDark: isDark,
      lattice: lattice,
    );
    // The field shader reads the coordinates before the scaling and moving, so
    // pixel i of the small image is the field at i grid cells. The first and
    // the last pixels sit on the edges of the drawing and beyond, so that
    // every point of the drawing lies between pixels and is interpolated, and
    // the field covers the whole image, so no pixel is partly transparent.
    final cell = max(1.0, max(size.width, size.height) / fieldSize);
    final width = (size.width / cell).ceil() + 1;
    final height = (size.height / cell).ceil() + 1;
    final recorder = ui.PictureRecorder();
    Canvas(recorder)
      ..scale(1 / cell)
      ..translate(cell / 2, cell / 2)
      ..drawRect(
        Rect.fromLTWH(-cell / 2, -cell / 2, width * cell, height * cell),
        Paint()..shader = _field,
      );
    final picture = recorder.endRecording();
    final field = picture.toImageSync(width, height);
    picture.dispose();

    _composite
      // resolution
      ..setFloat(0, size.width)
      ..setFloat(1, size.height)
      // u_field_scale
      ..setFloat(2, size.width / (width * cell))
      ..setFloat(3, size.height / (height * cell))
      // u_field_offset
      ..setFloat(4, 0.5 / width)
      ..setFloat(5, 0.5 / height)
      // u_is_dark
      ..setFloat(6, isDark ? 1 : 0)
      ..setImageSampler(0, covers)
      ..setImageSampler(1, field, filterQuality: FilterQuality.low);
    canvas.drawRect(Offset.zero & size, Paint()..shader = _composite);

    _lastField?.dispose();
    _lastField = field;
  }

  void dispose() {
    _lastField?.dispose();
    _field.dispose();
    _composite.dispose();
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
  late final Future<CoverWallGradient> _gradient;

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
    _gradient = CoverWallGradient.load();
    _gradient.then((gradient) {
      if (mounted) {
        _ticker.start();
      } else {
        gradient.dispose();
      }
    }, onError: (_) {});

    // Listen to mouse position updates
    WidgetsBinding.instance.pointerRouter.addGlobalRoute(_updateMousePosition);
  }

  void _onTick(Duration elapsed) {
    // A tenth of a unit each second.
    _time.value = elapsed.inMicroseconds / Duration.microsecondsPerSecond / 10;
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
    _gradient.then((gradient) => gradient.dispose(), onError: (_) {});
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return FutureBuilder<CoverWallGradient>(
      future: _gradient,
      builder: (context, snapshot) {
        if (!snapshot.hasData) {
          return widget.child;
        }

        final isDark = FluentTheme.of(context).brightness.isDark;
        // The noise lattice the app computes covers a scale below 2 (see
        // NoiseLattice).
        assert(widget.effectParams.scale < 2);

        final gradient = snapshot.data!;

        return ShaderSampler(
          (ui.Image image, Size size, Canvas canvas) {
            final renderBox =
                _containerKey.currentContext?.findRenderObject() as RenderBox?;
            final globalMousePosition = _globalMousePosition.value;
            final mousePosition =
                globalMousePosition != null && renderBox != null
                ? renderBox.globalToLocal(globalMousePosition)
                : Offset.zero;
            gradient.paint(
              canvas,
              size,
              image,
              time: _time.value,
              mouse: mousePosition,
              gradientParams: widget.gradientParams,
              effectParams: widget.effectParams,
              color: widget.color,
              color2: widget.color2,
              isDark: isDark,
              lattice: _lattice,
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
