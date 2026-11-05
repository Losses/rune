import 'dart:math' as math;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';

/// Two presses within this are a double press.
const _doublePress = Duration(milliseconds: 350);

/// How thick the splitter is, across the way it drags: the whole space
/// between the two parts it stands between, its grip in the middle.
const labSplitterThickness = 10.0;

/// A thin bar between two parts of the lab. Dragging it along [axis] sets the
/// size of the part before it: down or up between the cards and the timeline
/// for the height of the cards, right or left between two cards for the width
/// of the first. A double press sets it back to automatic.
///
/// It answers the pointer with its grip alone, and leaves the space it fills
/// as it is: under the pointer the grip is longer and brighter, and pressed
/// it takes the accent.
class LabSplitter extends StatefulWidget {
  const LabSplitter({
    super.key,
    this.axis = Axis.vertical,
    required this.currentSize,
    required this.minSize,
    required this.maxSize,
    required this.onSize,
  });

  /// The way it drags: vertical for a height, horizontal for a width.
  final Axis axis;

  /// The size of the part before it now, where a drag starts from.
  final double Function() currentSize;

  /// The least and the most the size can be dragged to.
  final double minSize;
  final double maxSize;

  /// Called with the new size while it is dragged, with [save] false, and
  /// once more at the end of the drag with [save] true. A null size is the
  /// automatic one.
  final void Function(double? size, {required bool save}) onSize;

  @override
  State<LabSplitter> createState() => _LabSplitterState();
}

class _LabSplitterState extends State<LabSplitter> {
  bool _hover = false;
  int? _pointer;
  double _start = 0;
  double _startSize = 0;
  double? _size;
  Duration? _lastDown;

  bool get _vertical => widget.axis == Axis.vertical;

  double _along(Offset position) => _vertical ? position.dy : position.dx;

  double _clamp(double s) =>
      math.max(widget.minSize, math.min(s, widget.maxSize));

  void _onDown(PointerDownEvent e) {
    if (e.buttons & kPrimaryButton == 0) return;
    final last = _lastDown;
    _lastDown = e.timeStamp;
    if (last != null && e.timeStamp - last <= _doublePress) {
      _lastDown = null;
      widget.onSize(null, save: true);
      return;
    }
    _pointer = e.pointer;
    _start = _along(e.position);
    _startSize = widget.currentSize();
    _size = null;
    setState(() {});
  }

  void _onMove(PointerMoveEvent e) {
    if (e.pointer != _pointer) return;
    final s = _clamp(_startSize + _along(e.position) - _start);
    _lastDown = null;
    _size = s;
    widget.onSize(s, save: false);
  }

  void _end() {
    final s = _size;
    _pointer = null;
    _size = null;
    setState(() {});
    // A press that did not move keeps the size, so it is not saved.
    if (s != null) widget.onSize(s, save: true);
  }

  @override
  Widget build(BuildContext context) {
    final grip = LabGrip(
      axis: _vertical ? Axis.horizontal : Axis.vertical,
      hover: _hover,
      pressed: _pointer != null,
    );
    return MouseRegion(
      cursor: _vertical
          ? SystemMouseCursors.resizeRow
          : SystemMouseCursors.resizeColumn,
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: Listener(
        behavior: HitTestBehavior.opaque,
        onPointerDown: _onDown,
        onPointerMove: _onMove,
        onPointerUp: (e) {
          if (e.pointer == _pointer) _end();
        },
        onPointerCancel: (e) {
          if (e.pointer == _pointer) _end();
        },
        // Across the way it drags it is as long as it is given; left to
        // itself, no longer than its grip, so that it sets no height of a
        // row it stands in.
        child: _vertical
            ? SizedBox(
                height: labSplitterThickness,
                child: Center(child: grip),
              )
            : SizedBox(
                width: labSplitterThickness,
                child: Center(heightFactor: 1, child: grip),
              ),
      ),
    );
  }
}

/// The pill that shows a splitter or a page handle: the only part of them that
/// is drawn. Under the pointer it is longer and brighter, and pressed it takes
/// the accent.
class LabGrip extends StatelessWidget {
  const LabGrip({
    super.key,
    required this.axis,
    this.hover = false,
    this.pressed = false,
  });

  /// The way the pill runs: horizontal for a wide one, on a bar that drags up
  /// and down.
  final Axis axis;
  final bool hover;
  final bool pressed;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final res = theme.resources;
    final hot = hover || pressed;
    final wide = axis == Axis.horizontal;
    return AnimatedContainer(
      duration: const Duration(milliseconds: 120),
      width: wide ? (hot ? 48 : 36) : 3,
      height: wide ? 3 : (hot ? 48 : 36),
      decoration: BoxDecoration(
        color: pressed
            ? theme.accentColor.defaultBrushFor(theme.brightness)
            : hover
            ? res.controlStrongFillColorDefault
            : res.controlStrongFillColorDisabled,
        borderRadius: BorderRadius.circular(1.5),
      ),
    );
  }
}
