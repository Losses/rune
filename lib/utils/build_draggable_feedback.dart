import 'package:fluent_ui/fluent_ui.dart';

Widget buildDraggableFeedback(
  BuildContext context,
  BoxConstraints constraints,
  Widget child,
) {
  return Transform(
    transform: Matrix4.rotationZ(0),
    alignment: FractionalOffset.topLeft,
    child: ConstrainedBox(
      constraints: constraints,
      child: child,
    ),
  );
}
