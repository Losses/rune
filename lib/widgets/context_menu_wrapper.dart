import 'package:fluent_ui/fluent_ui.dart';

import '../utils/platform.dart';

class ContextMenuWrapper extends StatelessWidget {
  final Widget child;
  final Function(Offset) onContextMenu;
  final Function(Offset) onMiddleClick;
  final GlobalKey contextAttachKey;
  final FlyoutController contextController;

  const ContextMenuWrapper({
    super.key,
    required this.child,
    required this.onContextMenu,
    required this.onMiddleClick,
    required this.contextAttachKey,
    required this.contextController,
  });

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onTertiaryTapUp: (event) => onMiddleClick(event.localPosition),
      onSecondaryTapUp:
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
