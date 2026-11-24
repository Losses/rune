import 'package:fluent_ui/fluent_ui.dart';

import '../../../router/router_aware_flyout_controller.dart';

/// A subtle button that opens a menu built when it is pressed, so a
/// folder's subfolders are read only when someone asks for them.
class FileDialogMenuButton extends StatefulWidget {
  const FileDialogMenuButton({
    super.key,
    required this.child,
    required this.items,
    this.tooltip,
    this.semanticLabel,
    this.placement = FlyoutPlacementMode.bottomLeft,
    this.padding,
  });

  final Widget child;
  final Future<List<MenuFlyoutItemBase>> Function() items;
  final String? tooltip;
  final String? semanticLabel;
  final FlyoutPlacementMode placement;
  final EdgeInsetsGeometry? padding;

  @override
  State<FileDialogMenuButton> createState() => FileDialogMenuButtonState();
}

class FileDialogMenuButtonState extends State<FileDialogMenuButton> {
  final _controller = RouterAwareFlyoutController();

  Future<void> open() async {
    final items = await widget.items();
    if (!mounted || items.isEmpty) return;
    await _controller.showFlyout(
      placementMode: widget.placement,
      builder: (context) => MenuFlyout(items: items),
    );
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final button = FlyoutTarget(
      controller: _controller.controller,
      child: IconButton(
        icon: widget.child,
        onPressed: open,
        style: widget.padding == null
            ? null
            : ButtonStyle(padding: WidgetStatePropertyAll(widget.padding)),
      ),
    );
    final labelled = Semantics(
      label: widget.semanticLabel ?? widget.tooltip,
      button: true,
      excludeSemantics: widget.semanticLabel != null,
      child: button,
    );
    final tooltip = widget.tooltip;
    return tooltip == null
        ? labelled
        : Tooltip(message: tooltip, child: labelled);
  }
}
