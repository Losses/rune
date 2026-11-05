import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/router/router_aware_flyout_controller.dart';

/// Shows a flyout of the lab the way the app's menus over the visualizer go
/// (the cover wall's is one): on the app's own navigator, as the page's is
/// under the playback controller and the navigation bar, which would cover
/// it, and in the theme of [context], as the flyout is outside the page (the
/// lab is dark whatever the app's theme is). A [position] is a global one, for
/// a menu at the pointer; without one the flyout stands by the controller's
/// target, as [placementMode] says.
Future<T?> showLabFlyout<T>(
  BuildContext context,
  RouterAwareFlyoutController controller, {
  required WidgetBuilder builder,
  FlyoutPlacementMode placementMode = FlyoutPlacementMode.auto,
  Offset? position,
}) {
  final theme = FluentTheme.of(context);
  final navigator = Navigator.of(context, rootNavigator: true);
  Offset? at;
  if (position != null) {
    final box = navigator.context.findRenderObject();
    at = box is RenderBox ? box.globalToLocal(position) : position;
  }
  return controller.showFlyout<T>(
    navigatorKey: navigator,
    placementMode: placementMode,
    position: at,
    builder: (context) => FluentTheme(data: theme, child: builder(context)),
  );
}

/// The item of a lab menu for one choice of several: the chosen one has a
/// check, as the cover wall's menu of scenes does. A [dim] one is shown at
/// less than full strength.
MenuFlyoutItem labChoiceItem({
  required String label,
  required bool chosen,
  required VoidCallback onPressed,
  bool dim = false,
}) {
  return MenuFlyoutItem(
    leading: chosen ? const Icon(Symbols.check) : null,
    text: Opacity(
      opacity: dim ? 0.45 : 1,
      child: Text(
        label,
        maxLines: 1,
        softWrap: false,
        overflow: TextOverflow.ellipsis,
      ),
    ),
    onPressed: onPressed,
  );
}

/// A list to choose one of [items] from: a button that shows the chosen
/// one's label, at its start, with an arrow at its end, and opens a menu of
/// them all under it, the chosen one checked. It is the lab's in place of
/// fluent's [ComboBox], whose list opens on the page's navigator, under the
/// playback controller, and looks unlike the app's menus.
class LabChoiceButton<T> extends StatefulWidget {
  const LabChoiceButton({
    super.key,
    required this.value,
    required this.items,
    required this.onChanged,
    this.isExpanded = false,
  });

  /// The chosen value.
  final T value;

  /// The values and their labels, in order.
  final List<(T, String)> items;
  final ValueChanged<T> onChanged;

  /// Whether the button takes the whole width it is given, rather than the
  /// width of its label.
  final bool isExpanded;

  @override
  State<LabChoiceButton<T>> createState() => _LabChoiceButtonState<T>();
}

class _LabChoiceButtonState<T> extends State<LabChoiceButton<T>> {
  final _controller = RouterAwareFlyoutController();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  void _open() {
    // The menu is at least as wide as the button.
    final width = context.size?.width ?? 0;
    showLabFlyout<void>(
      context,
      _controller,
      placementMode: FlyoutPlacementMode.bottomLeft,
      builder: (context) => MenuFlyout(
        constraints: BoxConstraints(minWidth: width),
        items: [
          for (final (value, label) in widget.items)
            labChoiceItem(
              label: label,
              chosen: value == widget.value,
              onPressed: () => widget.onChanged(value),
            ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final label = [
      for (final (value, label) in widget.items)
        if (value == widget.value) label,
    ].firstOrNull;
    final text = Text(
      label ?? '',
      maxLines: 1,
      softWrap: false,
      overflow: TextOverflow.ellipsis,
      textAlign: TextAlign.start,
    );
    return FlyoutTarget(
      controller: _controller.controller,
      child: Button(
        onPressed: _open,
        child: Row(
          mainAxisSize: widget.isExpanded ? MainAxisSize.max : MainAxisSize.min,
          children: [
            widget.isExpanded ? Expanded(child: text) : Flexible(child: text),
            const SizedBox(width: 8),
            const ChevronDown(),
          ],
        ),
      ),
    );
  }
}
