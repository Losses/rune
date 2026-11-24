import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';

/// A divider dragged sideways, such as the edge of the navigation pane or of
/// a details column. It reports how far the pointer has moved since it went
/// down, so the divider stays under the pointer and no move is lost when
/// several arrive between two frames.
class FileDialogDragHandle extends StatefulWidget {
  const FileDialogDragHandle({
    super.key,
    required this.onStart,
    required this.onDrag,
    required this.child,
    this.onEnd,
    this.onDoubleTap,
  });

  /// The drag began; the owner notes the size it starts from.
  final VoidCallback onStart;

  /// The pointer is [distance] logical pixels past where it went down,
  /// towards the end of the line in right-to-left text too.
  final void Function(double distance) onDrag;
  final VoidCallback? onEnd;
  final VoidCallback? onDoubleTap;
  final Widget child;

  @override
  State<FileDialogDragHandle> createState() => _FileDialogDragHandleState();
}

class _FileDialogDragHandleState extends State<FileDialogDragHandle> {
  double _distance = 0;

  @override
  Widget build(BuildContext context) {
    final rtl = Directionality.of(context) == TextDirection.rtl;
    return MouseRegion(
      cursor: SystemMouseCursors.resizeColumn,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        dragStartBehavior: DragStartBehavior.down,
        onHorizontalDragStart: (_) {
          _distance = 0;
          widget.onStart();
        },
        onHorizontalDragUpdate: (details) {
          _distance += rtl ? -details.delta.dx : details.delta.dx;
          widget.onDrag(_distance);
        },
        onHorizontalDragEnd: (_) => widget.onEnd?.call(),
        onDoubleTap: widget.onDoubleTap,
        child: widget.child,
      ),
    );
  }
}
