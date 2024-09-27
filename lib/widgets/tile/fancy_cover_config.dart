import 'package:fluent_ui/fluent_ui.dart';

class FancyCoverConfig {
  final double fontSize;
  final FontWeight fontWeight;
  final TextAlign textAlign;
  final bool toUpperCase;
  final double textBoxWidth;
  final Offset transformOrigin;
  final Offset position;
  final double rotation;

  const FancyCoverConfig({
    required this.fontSize,
    this.fontWeight = FontWeight.normal,
    this.textAlign = TextAlign.center,
    this.toUpperCase = false,
    this.textBoxWidth = double.infinity,
    this.transformOrigin = Offset.zero,
    required this.position,
    this.rotation = 0,
  });
}
