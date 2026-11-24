import 'package:desktop_drop/desktop_drop.dart';
import 'package:flutter/widgets.dart';

/// Keeps native drop coordinates in the view's coordinate space before the
/// plugin converts them through the render tree's application-scale transform.
class AppDropTarget extends StatelessWidget {
  const AppDropTarget({super.key, required this.child, this.onDragDone});

  final Widget child;
  final OnDragDoneCallback? onDragDone;

  @override
  Widget build(BuildContext context) {
    final applicationMedia = MediaQuery.of(context);
    return MediaQuery(
      data: applicationMedia.copyWith(
        devicePixelRatio: View.of(context).devicePixelRatio,
      ),
      child: DropTarget(
        onDragDone: onDragDone,
        child: MediaQuery(data: applicationMedia, child: child),
      ),
    );
  }
}
