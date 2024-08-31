import 'package:fluent_ui/fluent_ui.dart';

import '../utils/platform.dart';

class ContextMenuWrapper extends StatelessWidget {
  final Widget child;
  final Function(Offset) onContextMenu;
  final GlobalKey contextAttachKey;
  final FlyoutController contextController;

  const ContextMenuWrapper({
    super.key,
    required this.child,
    required this.onContextMenu,
    required this.contextAttachKey,
    required this.contextController,
  });

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onSecondaryTapDown:
          isDesktop ? (details) => onContextMenu(details.localPosition) : null,
      onLongPressEnd:
          isDesktop ? null : (details) => onContextMenu(details.localPosition),
      child: FlyoutTarget(
        key: contextAttachKey,
        controller: contextController,
        child: child,
      ),
    );
  }
}
