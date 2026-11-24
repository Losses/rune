import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/router/router_aware_flyout_controller.dart';
import 'lab_data.dart';
import 'lab_display.dart';
import 'lab_menu.dart';
import 'lab_segmented.dart';
import 'lab_views.dart';

const _flyoutWidth = 300.0;

/// The Display button of the timeline's header, which opens a flyout with how
/// the rows are drawn: which strips, whether levels are lines, and whether
/// rows with nothing in the song are left out. The choices apply at once and
/// belong to no view.
class LabDisplayButton extends StatefulWidget {
  const LabDisplayButton({
    super.key,
    required this.model,
    this.columns = const [],
    this.empty = const {},
  });

  final LabRowsModel model;

  /// The song's columns and the ids of those with nothing in it, as of now:
  /// the flyout counts the rows that hiding the empty ones leaves out.
  final List<LabColumn> columns;
  final Set<String> empty;

  @override
  State<LabDisplayButton> createState() => _LabDisplayButtonState();
}

class _LabDisplayButtonState extends State<LabDisplayButton> {
  final _controller = RouterAwareFlyoutController();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  void _open() {
    showLabFlyout<void>(
      context,
      _controller,
      placementMode: FlyoutPlacementMode.bottomRight,
      builder: (context) => FlyoutContent(
        constraints: const BoxConstraints(maxWidth: _flyoutWidth),
        child: LabDisplayContent(
          model: widget.model,
          columns: widget.columns,
          empty: widget.empty,
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return FlyoutTarget(
      controller: _controller.controller,
      child: Tooltip(
        message: 'Display',
        child: Button(
          onPressed: _open,
          child: const Icon(Symbols.tune, size: 16),
        ),
      ),
    );
  }
}

/// The inside of the Display flyout.
class LabDisplayContent extends StatelessWidget {
  const LabDisplayContent({
    super.key,
    required this.model,
    this.columns = const [],
    this.empty = const {},
  });

  final LabRowsModel model;
  final List<LabColumn> columns;
  final Set<String> empty;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: model,
      builder: (context, _) {
        final display = model.display;
        final selection = model.selection;
        // What the timeline leaves out as empty: the pinned rows stay.
        final left = display.hideEmpty
            ? columns
                  .where(
                    (c) =>
                        selection.isShown(c) &&
                        empty.contains(c.id) &&
                        !selection.isPinned(c.id),
                  )
                  .length
            : 0;
        return Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            InfoLabel(
              label: 'Strips',
              child: LabSegmented<LabStripMode>(
                value: display.stripMode,
                items: const [
                  (LabStripMode.plan, 'Plan'),
                  (LabStripMode.actual, 'Actual'),
                  (LabStripMode.both, 'Both'),
                ],
                onChanged: (mode) =>
                    model.setDisplay(display.withStripMode(mode)),
              ),
            ),
            const SizedBox(height: 12),
            _switch(
              label: 'Draw Levels as Lines',
              checked: display.allLines,
              onChanged: (on) => model.setDisplay(display.withAllLines(on)),
            ),
            const SizedBox(height: 12),
            _switch(
              label: left > 0
                  ? 'Hide Rows with Nothing in This Song ($left)'
                  : 'Hide Rows with Nothing in This Song',
              checked: display.hideEmpty,
              onChanged: (on) => model.setDisplay(display.withHideEmpty(on)),
            ),
          ],
        );
      },
    );
  }
}

/// A switch at the right edge with its label on the left, which a tap on the
/// label also turns, as the app's mix dialog has them.
Widget _switch({
  required String label,
  required bool checked,
  required ValueChanged<bool> onChanged,
}) {
  return ToggleSwitch(
    checked: checked,
    onChanged: onChanged,
    leadingContent: true,
    content: Expanded(
      child: Text(label, maxLines: 2, overflow: TextOverflow.ellipsis),
    ),
  );
}
