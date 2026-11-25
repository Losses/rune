import 'package:fluent_ui/fluent_ui.dart';

Size parseSize(String input) {
  List<String> parts = input.split(' ');

  int widthRatio = int.parse(parts[0]);
  int heightRatio = int.parse(parts[1]);

  int width = 1920;
  int height = (1920 * heightRatio / widthRatio).round();

  return Size(width.toDouble(), height.toDouble());
}
