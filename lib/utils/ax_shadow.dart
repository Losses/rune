import 'package:fluent_ui/fluent_ui.dart';

List<BoxShadow> axShadow(double elevationDepth) {
  return [
    BoxShadow(
      color: Color.fromRGBO(
        0,
        0,
        0,
        (0.13441 -
            0.00063 * elevationDepth +
            0.00003 * elevationDepth * elevationDepth),
      ),
      offset: Offset(
        0,
        0.400 * elevationDepth,
      ),
      blurRadius: 0.900 * elevationDepth,
    ),
    BoxShadow(
      color: Color.fromRGBO(
        0,
        0,
        0,
        (0.10997 -
            0.00051 * elevationDepth +
            0.00003 * elevationDepth * elevationDepth),
      ),
      offset: Offset(
        0,
        0.075 * elevationDepth,
      ),
      blurRadius: 0.225 * elevationDepth,
    ),
  ];
}
