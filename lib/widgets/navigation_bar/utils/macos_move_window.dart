import 'package:bitsdojo_window/bitsdojo_window.dart';
import 'package:fluent_ui/fluent_ui.dart';

class MacOSMoveWindow extends StatelessWidget {
  final Widget? child;
  final bool isEnabledDoubleTap;
  final VoidCallback? onDoubleTap;

  MacOSMoveWindow({
    Key? key,
    this.child,
    this.isEnabledDoubleTap = true,
    this.onDoubleTap,
  }) : super(key: key);

  @override
  Widget build(BuildContext context) {
    if (child == null) {
      return _MacOSMoveWindow(
          isEnabledDoubleTap: isEnabledDoubleTap,
          onDoubleTap: onDoubleTap);
    }
    return _MacOSMoveWindow(
      isEnabledDoubleTap: isEnabledDoubleTap,
      onDoubleTap: onDoubleTap,
      child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [Expanded(child: child!)]),
    );
  }
}

class _MacOSMoveWindow extends StatelessWidget {
  _MacOSMoveWindow({
    Key? key,
    this.child,
    this.isEnabledDoubleTap = true,
    this.onDoubleTap,
  }) : super(key: key);
  final Widget? child;
  final bool isEnabledDoubleTap;
  final VoidCallback? onDoubleTap;
  @override
  Widget build(BuildContext context) {
    return GestureDetector(
        behavior: HitTestBehavior.opaque,
        onPanStart: (details) {
          appWindow.startDragging();
        },
        onDoubleTap: isEnabledDoubleTap
            ? this.onDoubleTap ?? () => appWindow.maximizeOrRestore()
            : null,
        child: this.child ?? Container());
  }
}
