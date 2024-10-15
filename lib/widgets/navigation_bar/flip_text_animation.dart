import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/lerp_controller.dart';

import './utils/text_style_sheet.dart';

class FlipTextAnimation extends StatefulWidget {
  final FlipTextPositions fromPositions;
  final FlipTextPositions toPositions;
  final FlipTextStyles fromStyles;
  final FlipTextStyles toStyles;
  final String text;
  final VoidCallback onAnimationComplete;

  const FlipTextAnimation({
    super.key,
    required this.fromPositions,
    required this.toPositions,
    required this.fromStyles,
    required this.toStyles,
    required this.text,
    required this.onAnimationComplete,
  });

  @override
  FlipTextAnimationState createState() => FlipTextAnimationState();
}

class FlipTextAnimationState extends State<FlipTextAnimation>
    with TickerProviderStateMixin {
  late LerpController _positionXController;
  late LerpController _positionYController;
  late LerpController _scaleController;
  late LerpController _alphaController;
  late LerpController _fontWeightController;

  late double x;
  late double y;
  late double scale;
  late double alpha;
  late double fontWeight;

  @override
  void initState() {
    super.initState();

    x = widget.fromPositions.position.dx;
    y = widget.fromPositions.position.dy;
    scale = widget.fromStyles.scale;
    alpha = widget.fromStyles.alpha?.toDouble() ?? 255.0;
    fontWeight =
        widget.fromStyles.fontWeight ?? FontWeight.normal.value.toDouble();

    _positionXController = LerpController(
      initialValue: x,
      getter: () => x,
      setter: (value) => setState(() {
        x = value;
      }),
      t: 0.15,
      vsync: this,
    );

    _positionYController = LerpController(
      initialValue: y,
      getter: () => y,
      setter: (value) => setState(() {
        y = value;
      }),
      t: 0.15,
      vsync: this,
    );

    _scaleController = LerpController(
      initialValue: scale,
      getter: () => scale,
      setter: (value) => setState(() {
        scale = value;
      }),
      t: 0.15,
      vsync: this,
    );

    _alphaController = LerpController(
      initialValue: alpha,
      getter: () => alpha,
      setter: (value) => setState(() {
        alpha = value;
      }),
      t: 0.15,
      vsync: this,
    );

    _fontWeightController = LerpController(
      initialValue: fontWeight,
      getter: () => fontWeight,
      setter: (value) => setState(() {
        fontWeight = value;
      }),
      t: 0.15,
      vsync: this,
    );

    _startAnimation();
  }

  Future<void> _startAnimation() async {
    List<Future<void>> futures = [
      _positionXController.lerp(widget.toPositions.position.dx),
      _positionYController.lerp(widget.toPositions.position.dy),
      _scaleController.lerp(widget.toStyles.scale),
      _alphaController.lerp(widget.toStyles.alpha?.toDouble() ?? 255),
      _fontWeightController.lerp(
          widget.toStyles.fontWeight ?? FontWeight.normal.value.toDouble()),
    ];

    await Future.wait(futures);

    widget.onAnimationComplete();
  }

  @override
  void dispose() {
    _positionXController.dispose();
    _positionYController.dispose();
    _scaleController.dispose();
    _alphaController.dispose();
    _fontWeightController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Positioned(
      left: x,
      top: y,
      child: Transform.scale(
        scale: scale,
        alignment: Alignment.topLeft,
        child: Text(
          widget.text,
          style: TextStyle(
            fontVariations: <FontVariation>[FontVariation('wght', fontWeight)],
            color: (widget.toStyles.color ?? widget.fromStyles.color)?.withAlpha(alpha.toInt()),
          ),
        ),
      ),
    );
  }
}
