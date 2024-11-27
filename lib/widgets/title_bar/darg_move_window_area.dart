import 'package:fluent_ui/fluent_ui.dart';
import 'package:bitsdojo_window/bitsdojo_window.dart';

class DargMoveWindowArea extends StatelessWidget {
  final Widget? child;

  const DargMoveWindowArea({super.key, this.child});

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onDoubleTap: () => appWindow.maximizeOrRestore(),
      onPanStart: (_) => appWindow.startDragging(),
      child: SizedBox(
        width: double.infinity,
        height: double.infinity,
        child: SizedBox.expand(
          child: child,
        ),
      ),
    );
  }
}
