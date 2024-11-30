import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:bitsdojo_window/bitsdojo_window.dart';

class DragMoveWindowArea extends StatelessWidget {
  final Widget? child;
  final bool isEnabledDoubleTap;

  const DragMoveWindowArea({super.key, this.child, this.isEnabledDoubleTap = true});

  @override
  Widget build(BuildContext context) {
    int tapCount = 0;
    Timer? tapTimer;

    void handleTap() {
      tapCount++;
      if (tapCount == 1) {
        tapTimer = Timer(const Duration(milliseconds: 300), () {
          tapCount = 0;
        });
      } else if (tapCount == 2) {
        tapTimer?.cancel();
        tapCount = 0;
        appWindow.maximizeOrRestore();
      }
    }

    return Listener(
      onPointerUp: (_) => isEnabledDoubleTap ? handleTap() : null,
      child: GestureDetector(
        onPanStart: (_) => appWindow.startDragging(),
        child: SizedBox(
          width: double.infinity,
          height: double.infinity,
          child: SizedBox.expand(
            child: Container(
              color: Colors.transparent,
              child: child,
            ),
          ),
        ),
      ),
    );
  }
}
