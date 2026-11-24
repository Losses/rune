import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/services.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../l10n.dart';

import '../file_dialog_controller.dart';
import '../file_dialog_model.dart';

const double _rowHeight = 32;
const double _trackWidth = 28;

/// The name File Explorer gives a named view.
String fileDialogViewLabel(BuildContext context, FileDialogView view) {
  final s = S.of(context);
  return switch (view.position.round()) {
    0 => s.fileDialogExtraLargeIcons,
    1 => s.fileDialogLargeIcons,
    2 => s.fileDialogMediumIcons,
    3 => s.fileDialogSmallIcons,
    4 => s.fileDialogList,
    5 => s.fileDialogDetails,
    6 => s.fileDialogTiles,
    _ => s.fileDialogContent,
  };
}

/// The icon of the named view nearest to [view].
IconData fileDialogViewIcon(FileDialogView view) =>
    switch (view.position.round()) {
      0 => Symbols.photo_size_select_actual,
      1 => Symbols.grid_view,
      2 => Symbols.apps,
      3 => Symbols.view_comfy,
      4 => Symbols.view_column,
      5 => Symbols.view_list,
      6 => Symbols.view_module,
      _ => Symbols.view_agenda,
    };

/// The view menu of the file dialog's view button, as Windows' file dialogs
/// show it: the eight named views beside a slider whose thumb marks the
/// current view. Dragging the thumb between Extra large and Medium icons
/// sizes the icons smoothly and shows the change at once; below that it
/// moves from view to view. Clicking a view picks it and closes the menu.
class FileDialogViewMenu extends StatefulWidget {
  const FileDialogViewMenu({super.key, required this.controller});

  final FileDialogController controller;

  @override
  State<FileDialogViewMenu> createState() => _FileDialogViewMenuState();
}

class _FileDialogViewMenuState extends State<FileDialogViewMenu> {
  FileDialogController get _controller => widget.controller;

  void _slideTo(double dy) {
    _controller.setView(FileDialogView.at((dy - _rowHeight / 2) / _rowHeight));
  }

  /// One named view up (negative) or down the slider.
  void _step(int delta) {
    final position = _controller.view.position;
    final target = delta < 0 ? position.ceil() - 1 : position.floor() + 1;
    _controller.setView(FileDialogView.at(target.toDouble()));
  }

  void _close() => Navigator.of(context).maybePop();

  KeyEventResult _onKey(FocusNode node, KeyEvent event) {
    if (event is! KeyDownEvent && event is! KeyRepeatEvent) {
      return KeyEventResult.ignored;
    }
    switch (event.logicalKey) {
      case LogicalKeyboardKey.arrowUp:
        _step(-1);
      case LogicalKeyboardKey.arrowDown:
        _step(1);
      case LogicalKeyboardKey.enter ||
          LogicalKeyboardKey.numpadEnter ||
          LogicalKeyboardKey.space:
        _close();
      default:
        return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: _controller,
      builder: (context, _) {
        final view = _controller.view;
        final nearest = FileDialogView.at(view.position.roundToDouble());
        final label = fileDialogViewLabel(context, nearest);
        final track = Semantics(
          slider: true,
          label: S.of(context).fileDialogView,
          value: label,
          increasedValue: view.position < 1
              ? null
              : fileDialogViewLabel(
                  context,
                  FileDialogView.at(view.position.ceil() - 1.0),
                ),
          decreasedValue: view.position >= FileDialogView.named.length - 1
              ? null
              : fileDialogViewLabel(
                  context,
                  FileDialogView.at(view.position.floor() + 1.0),
                ),
          onIncrease: view.position < 1 ? null : () => _step(-1),
          onDecrease: view.position >= FileDialogView.named.length - 1
              ? null
              : () => _step(1),
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTapDown: (details) => _slideTo(details.localPosition.dy),
            onVerticalDragUpdate: (details) =>
                _slideTo(details.localPosition.dy),
            child: CustomPaint(
              size: Size(_trackWidth, _rowHeight * FileDialogView.named.length),
              painter: _SliderPainter(
                position: view.position,
                stops: FileDialogView.named.length,
                theme: FluentTheme.of(context),
              ),
            ),
          ),
        );
        return Focus(
          autofocus: true,
          onKeyEvent: _onKey,
          child: FlyoutContent(
            padding: const EdgeInsets.symmetric(vertical: 4),
            child: IntrinsicWidth(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Row(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      track,
                      Expanded(
                        child: Column(
                          mainAxisSize: MainAxisSize.min,
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: [
                            for (final named in FileDialogView.named)
                              _ViewRow(
                                icon: fileDialogViewIcon(named),
                                label: fileDialogViewLabel(context, named),
                                current: named == view,
                                onPressed: () {
                                  _controller.setView(named);
                                  _close();
                                },
                              ),
                          ],
                        ),
                      ),
                    ],
                  ),
                  // Below the views, as a menu's separator draws it.
                  const Padding(
                    padding: EdgeInsets.symmetric(vertical: 4),
                    child: Divider(
                      style: DividerThemeData(
                        horizontalMargin: EdgeInsets.zero,
                      ),
                    ),
                  ),
                  _HiddenItemsRow(
                    shown: _controller.showHidden,
                    label: S.of(context).fileDialogHiddenItems,
                    onPressed: () {
                      _controller.setShowHidden(!_controller.showHidden);
                      _close();
                    },
                  ),
                ],
              ),
            ),
          ),
        );
      },
    );
  }
}

class _ViewRow extends StatelessWidget {
  const _ViewRow({
    required this.icon,
    required this.label,
    required this.current,
    required this.onPressed,
  });

  final IconData icon;
  final String label;
  final bool current;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    return HoverButton(
      onPressed: onPressed,
      semanticLabel: label,
      focusEnabled: false,
      builder: (context, states) => Semantics(
        selected: current,
        child: Container(
          height: _rowHeight,
          margin: const EdgeInsetsDirectional.only(end: 4),
          padding: const EdgeInsetsDirectional.only(start: 8, end: 16),
          decoration: BoxDecoration(
            color: ButtonThemeData.uncheckedInputColor(
              theme,
              states,
              transparentWhenNone: true,
            ),
            borderRadius: BorderRadius.circular(4),
          ),
          child: Row(
            children: [
              Icon(icon, size: 16),
              const SizedBox(width: 12),
              Text(label),
            ],
          ),
        ),
      ),
    );
  }
}

/// Whether hidden items show: a check mark under the slider's track when
/// they do, and the label lined up with the views' names.
class _HiddenItemsRow extends StatelessWidget {
  const _HiddenItemsRow({
    required this.shown,
    required this.label,
    required this.onPressed,
  });

  final bool shown;
  final String label;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    return HoverButton(
      onPressed: onPressed,
      semanticLabel: label,
      focusEnabled: false,
      builder: (context, states) => Semantics(
        toggled: shown,
        child: Container(
          height: _rowHeight,
          margin: const EdgeInsetsDirectional.only(start: 4, end: 4),
          decoration: BoxDecoration(
            color: ButtonThemeData.uncheckedInputColor(
              theme,
              states,
              transparentWhenNone: true,
            ),
            borderRadius: BorderRadius.circular(4),
          ),
          child: Row(
            children: [
              SizedBox(
                width: _trackWidth - 4,
                child: shown ? const Icon(Symbols.check, size: 16) : null,
              ),
              // Where the views' icons are, so the label lines up with
              // their names.
              const SizedBox(width: 8 + 16 + 12),
              Padding(
                padding: const EdgeInsetsDirectional.only(end: 16),
                child: Text(label),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// The view slider: a track from the first view to the last, a tick at
/// each named view, and the thumb at [position], drawn as Fluent's slider
/// thumb.
class _SliderPainter extends CustomPainter {
  const _SliderPainter({
    required this.position,
    required this.stops,
    required this.theme,
  });

  final double position;
  final int stops;
  final FluentThemeData theme;

  @override
  void paint(Canvas canvas, Size size) {
    final x = size.width / 2;
    double y(double at) => _rowHeight / 2 + at * _rowHeight;
    final line = Paint()
      ..color = theme.resources.controlStrongStrokeColorDefault
      ..strokeWidth = 2
      ..strokeCap = StrokeCap.round;
    canvas.drawLine(Offset(x, y(0)), Offset(x, y(stops - 1.0)), line);
    final tick = Paint()
      ..color = theme.resources.controlStrongStrokeColorDefault
      ..strokeWidth = 1;
    for (var stop = 0; stop < stops; stop++) {
      canvas.drawLine(
        Offset(x + 4, y(stop.toDouble())),
        Offset(x + 8, y(stop.toDouble())),
        tick,
      );
    }
    final center = Offset(x, y(position));
    canvas.drawCircle(
      center,
      9,
      Paint()..color = theme.resources.controlSolidFillColorDefault,
    );
    canvas.drawCircle(
      center,
      9,
      Paint()
        ..color = theme.resources.controlStrokeColorDefault
        ..style = PaintingStyle.stroke,
    );
    canvas.drawCircle(
      center,
      5,
      Paint()..color = theme.accentColor.defaultBrushFor(theme.brightness),
    );
  }

  @override
  bool shouldRepaint(_SliderPainter oldDelegate) =>
      oldDelegate.position != position ||
      oldDelegate.stops != stops ||
      oldDelegate.theme != theme;
}
