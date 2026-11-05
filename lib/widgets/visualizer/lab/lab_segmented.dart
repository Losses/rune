import 'package:fluent_ui/fluent_ui.dart';

/// One choice of a few, as buttons joined in one frame: a single border
/// around them all, a line between each two, and the chosen one filled the
/// way a checked toggle button is. fluent_ui has no segmented control, so
/// this is made of its own colors.
class LabSegmented<T> extends StatelessWidget {
  const LabSegmented({
    super.key,
    required this.value,
    required this.items,
    required this.onChanged,
    this.height = 30,
  });

  /// The chosen value.
  final T value;

  /// The values and their labels, in order.
  final List<(T, String)> items;
  final ValueChanged<T> onChanged;
  final double height;

  @override
  Widget build(BuildContext context) {
    final stroke = FluentTheme.of(context).resources.controlStrokeColorDefault;
    return _Frame(
      stroke: stroke,
      height: height,
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (var i = 0; i < items.length; i++) ...[
            if (i > 0)
              ColoredBox(color: stroke, child: const SizedBox(width: 1)),
            Expanded(
              child: _Segment(
                label: items[i].$2,
                selected: items[i].$1 == value,
                onPressed: () => onChanged(items[i].$1),
              ),
            ),
          ],
        ],
      ),
    );
  }
}

// The rounding of a frame, as fluent's buttons have.
const _radius = 4.0;
const _strokeWidth = 1.0;

/// The frame of joined buttons: a rounded border, and what is in it clipped
/// to the border's inner edge, so that a filled first or last button rounds
/// with the frame, as a button's fill does with its border. Clipped to the
/// outer edge instead, the fill would cover the border's curve at the
/// corners and stand square in them.
class _Frame extends StatelessWidget {
  const _Frame({required this.stroke, this.height, required this.child});

  final Color stroke;
  final double? height;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    return Container(
      height: height,
      decoration: BoxDecoration(
        border: Border.all(color: stroke, width: _strokeWidth),
        borderRadius: BorderRadius.circular(_radius),
      ),
      child: ClipRRect(
        borderRadius: BorderRadius.circular(_radius - _strokeWidth),
        child: child,
      ),
    );
  }
}

class _Segment extends StatelessWidget {
  const _Segment({
    required this.label,
    required this.selected,
    required this.onPressed,
  });

  final String label;
  final bool selected;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    return Semantics(
      button: true,
      selected: selected,
      inMutuallyExclusiveGroup: true,
      child: HoverButton(
        onPressed: onPressed,
        builder: (context, states) => ColoredBox(
          color: selected
              ? ButtonThemeData.checkedInputColor(theme, states)
              : ButtonThemeData.buttonColor(context, states),
          child: Center(
            child: Text(
              label,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                fontSize: 12,
                color: selected
                    ? theme.resources.textOnAccentFillColorPrimary
                    : ButtonThemeData.buttonForegroundColor(context, states),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// One button of a [LabButtonGroup]: what it shows, what it does, and a
/// [width] for what it shows, so that a label that changes keeps the group
/// still.
class LabGroupButton {
  const LabGroupButton({
    required this.child,
    required this.onPressed,
    this.tooltip,
    this.width,
  });

  final Widget child;
  final VoidCallback onPressed;
  final String? tooltip;
  final double? width;
}

/// Buttons that act together joined in one frame, as [LabSegmented]'s choices
/// are: a single border around them, a line between each two, each as tall
/// and padded as a button and coloured the way one is.
class LabButtonGroup extends StatelessWidget {
  const LabButtonGroup({super.key, required this.buttons});

  final List<LabGroupButton> buttons;

  @override
  Widget build(BuildContext context) {
    final stroke = FluentTheme.of(context).resources.controlStrokeColorDefault;
    return _Frame(
      stroke: stroke,
      child: IntrinsicHeight(
        child: Row(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (var i = 0; i < buttons.length; i++) ...[
              if (i > 0)
                ColoredBox(color: stroke, child: const SizedBox(width: 1)),
              _GroupButton(button: buttons[i]),
            ],
          ],
        ),
      ),
    );
  }
}

class _GroupButton extends StatelessWidget {
  const _GroupButton({required this.button});

  final LabGroupButton button;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final width = button.width;
    Widget result = HoverButton(
      onPressed: button.onPressed,
      builder: (context, states) {
        final foreground = ButtonThemeData.buttonForegroundColor(
          context,
          states,
        );
        return ColoredBox(
          color: ButtonThemeData.buttonColor(context, states),
          child: Padding(
            // A button's padding; the frame draws its border.
            padding: kDefaultButtonPadding,
            child: DefaultTextStyle.merge(
              style: (theme.typography.body ?? const TextStyle()).copyWith(
                color: foreground,
              ),
              child: IconTheme.merge(
                data: IconThemeData(color: foreground, size: 16),
                child: width == null
                    ? Center(widthFactor: 1, child: button.child)
                    : SizedBox(
                        width: width,
                        child: Center(child: button.child),
                      ),
              ),
            ),
          ),
        );
      },
    );
    if (button.tooltip != null) {
      result = Tooltip(message: button.tooltip, child: result);
    }
    return result;
  }
}
