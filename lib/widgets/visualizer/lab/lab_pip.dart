import 'dart:async';
import 'dart:math' as math;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/physics.dart';
import 'package:flutter/services.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/ax_shadow.dart';
import '../visualizer_controller.dart';
import 'lab_overlay.dart';
import 'lab_pip_layout.dart';

/// How far the picture keeps inside the pages' edges ([LabPip.insets]), so that
/// there is room around it however big it is.
const labPipMargin = 12.0;

/// How far it stands off the pages, by the app's own shadows: as high as the
/// welcome card floats over the home page.
const _pipElevation = 20.0;

/// The picture is this part of the overlay's shorter side wide, within the
/// least and the most.
const _pipWidthFraction = 0.4;
const _pipMinWidth = 120.0;
const _pipMaxWidth = 240.0;

/// How much of it stays on screen when it is tucked into a side.
const _pipTab = 28.0;

/// A flick carries it as far as it would slide before stopping, as the system
/// picture-in-picture works it out: this many seconds of the release speed.
const _flingHorizon = 0.5;

/// A flick this much more sideways than up or down can tuck it into a side;
/// a slanted one goes to a corner.
const _sidewaysRatio = 1.5;

/// The settle is a spring that is just not bouncy. It starts with the speed
/// the finger left with and is clamped at the target so that it never goes
/// past the corner. It is done when it is within half a pixel.
final _settleSpring = SpringDescription.withDampingRatio(
  mass: 1,
  stiffness: 240,
  ratio: 1,
);
const _settleTolerance = Tolerance(distance: 0.5, velocity: 5);

/// How fast the wheel resizes it: the width is multiplied by e to the power of
/// this times the wheel's travel in pixels.
const _wheelZoomRate = 0.002;

/// How much of its free corner takes a mouse to resize it.
const _pipCornerGrip = 12.0;

/// A wheel that stops for this long has let the picture go.
const _wheelIdle = Duration(milliseconds: 250);

/// Takes only the trackpad's two-finger gestures, and takes them at once, so
/// that the pages under the picture neither scroll nor zoom with them.
class _TrackpadRecognizer extends ScaleGestureRecognizer {
  _TrackpadRecognizer({super.debugOwner});

  @override
  bool isPointerAllowed(PointerDownEvent event) => false;

  @override
  void addAllowedPointerPanZoom(PointerPanZoomStartEvent event) {
    super.addAllowedPointerPanZoom(event);
    resolve(GestureDisposition.accepted);
  }
}

enum _Trackpad { none, pan, zoom }

/// The picture of the scene floating over the pages of a small window, as a
/// phone's picture-in-picture is. A finger or the mouse drags it, and a flick
/// sends it to the corner it points at; dragged or flicked off a side it is
/// tucked away, with only a tab left to bring it back by. Two fingers, a
/// trackpad pinch or the wheel with Ctrl or Cmd resize it, and a double tap
/// switches between its default and its most width. Held still for a long
/// press it peeks, until it is let go; a press that moves first drags and
/// never peeks.
class LabPip extends StatefulWidget {
  const LabPip({
    super.key,
    required this.controller,
    required this.area,
    required this.insets,
    required this.layout,
    required this.onLayout,
    required this.onHold,
  });

  final VisualizerController controller;

  /// The size of the overlay it floats in.
  final Size area;

  /// Where the pages' content starts inside the overlay on each side: their
  /// padding and what the screen takes (a notch, a home bar).
  final EdgeInsets insets;

  /// Where it rests and how wide it is.
  final LabPipLayout layout;

  /// Called when it is let go, with where it has gone and how wide it is.
  final ValueChanged<LabPipLayout> onLayout;
  final ValueChanged<bool> onHold;

  @override
  State<LabPip> createState() => _LabPipState();
}

class _LabPipState extends State<LabPip> with TickerProviderStateMixin {
  // Where it is while it is held, flicked or settling, one axis each. When
  // neither is in use it sits where its layout puts it.
  late final AnimationController _sx = AnimationController.unbounded(
    vsync: this,
  );
  late final AnimationController _sy = AnimationController.unbounded(
    vsync: this,
  );
  // Where the settle runs from and to, so that it is held at the target.
  (double, double)? _boundX;
  (double, double)? _boundY;
  bool _free = false;
  // Set while a settle starts: the controllers change status as they are
  // given their values, before either runs, and that is not a settle's end.
  bool _starting = false;

  // The double tap's change of width.
  late final AnimationController _zoom = AnimationController(vsync: this);
  double _zoomFrom = 0;

  final _pointers = <int, Offset>{};
  VelocityTracker? _tracker;
  Offset _down = Offset.zero;
  Offset _origin = Offset.zero;
  Offset _startAt = Offset.zero;
  Offset _shown = Offset.zero;
  Size _size = Size.zero;
  bool _dragging = false;
  bool _held = false;
  Timer? _timer;

  // A resize in progress, by two fingers, a trackpad or the wheel: the width
  // it has now, and where it is scaled from. When it was not at rest the
  // corner of its box it is scaled from is held; at rest the layout's own
  // corner is.
  bool _pinching = false;
  double _pinchDistance = 1;
  double _pinchWidth = 0;
  double? _liveWidth;
  Rect? _resizeFrom;
  Alignment _anchor = Alignment.bottomRight;
  Timer? _wheelTimer;

  // A mouse drag from the free corner resizes it.
  bool _cornerDrag = false;
  bool _cornerHover = false;
  double _cornerWidth = 0;
  double _shape = 16 / 9;

  // A two-finger gesture on a trackpad: it moves the picture or scales it,
  // whichever it starts as.
  _Trackpad _trackpad = _Trackpad.none;
  bool _trackpadOn = false;
  Offset _trackpadPan = Offset.zero;

  // Where the last tap was, until a second one is too late to make a double.
  Offset? _lastTap;
  Timer? _tapTimer;

  // The screen's own edges, taken in the last build.
  EdgeInsets _viewPadding = EdgeInsets.zero;

  double _minWidth = _pipMinWidth;
  double _maxWidth = _pipMaxWidth;

  @override
  void initState() {
    super.initState();
    _sx.addStatusListener((_) => _settled());
    _sy.addStatusListener((_) => _settled());
  }

  @override
  void dispose() {
    _timer?.cancel();
    _wheelTimer?.cancel();
    _tapTimer?.cancel();
    _sx.dispose();
    _sy.dispose();
    _zoom.dispose();
    super.dispose();
  }

  bool get _stashed => widget.layout.stash != LabPipSide.none;

  // Back to following its layout once the spring is done.
  void _settled() {
    if (!_free ||
        _starting ||
        _sx.isAnimating ||
        _sy.isAnimating ||
        _pointers.isNotEmpty ||
        _dragging ||
        _trackpadOn ||
        _wheelTimer?.isActive == true) {
      return;
    }
    setState(() => _free = false);
  }

  // The top and bottom of the room it moves in.
  (double, double) _vertical(Size size) {
    final top = labPipMargin + widget.insets.top;
    final bottom = math.max(
      top,
      widget.area.height - labPipMargin - widget.insets.bottom - size.height,
    );
    return (top, bottom);
  }

  Offset _rest(LabPipLayout layout, Size size) {
    final insets = widget.insets;
    final area = widget.area;
    final (top, bottom) = _vertical(size);
    if (layout.stash != LabPipSide.none) {
      return Offset(
        _tuckedX(layout.stash, size.width),
        top + (bottom - top) * layout.stashAt,
      );
    }
    final left = labPipMargin + insets.left;
    final right = area.width - labPipMargin - insets.right - size.width;
    return Offset(
      layout.corner.x < 0 ? left : math.max(left, right),
      layout.corner.y < 0 ? top : bottom,
    );
  }

  // Where a picture tucked into [side] has its left edge: its tab at the
  // screen's edge, clear of what the screen takes there (a notch).
  double _tuckedX(LabPipSide side, [double? width]) {
    final w = width ?? _size.width;
    return side == LabPipSide.left
        ? _viewPadding.left + _pipTab - w
        : widget.area.width - _viewPadding.right - _pipTab;
  }

  // The corner on the side of the overlay [point] is in.
  Alignment _quadrant(Offset point) {
    final insets = widget.insets;
    final area = widget.area;
    final inner = Rect.fromLTRB(
      insets.left,
      insets.top,
      area.width - insets.right,
      area.height - insets.bottom,
    );
    return Alignment(
      point.dx < inner.center.dx ? -1 : 1,
      point.dy < inner.center.dy ? -1 : 1,
    );
  }

  void _onDown(PointerDownEvent e) {
    if (e.buttons & kPrimaryButton == 0 || _held || _pointers.length >= 2) {
      return;
    }
    if (_pointers.isEmpty) {
      _dragging = false;
      _pinching = false;
      // A wheel resize that has not timed out is over, so that it is saved.
      if (_wheelTimer?.isActive == true) _endResize();
      _wheelTimer?.cancel();
      // Taken where it is now, even if it is still settling.
      _sx.stop();
      _sy.stop();
      _down = e.position;
      _startAt = _shown;
      _tracker = VelocityTracker.withKind(e.kind)
        ..addPosition(e.timeStamp, e.position);
      if (e.kind == PointerDeviceKind.mouse && _inFreeCorner(e.localPosition)) {
        _cornerDrag = true;
        _cornerWidth = _size.width;
        _beginResize();
      } else if (!_stashed) {
        // A tucked picture does not peek: its press is a tap or a drag.
        _timer = Timer(kLongPressTimeout, () {
          _held = true;
          widget.onHold(true);
        });
      }
    }
    _pointers[e.pointer] = e.position;
    if (_pointers.length == 2 && !_stashed && !_cornerDrag) _beginPinch();
  }

  void _onMove(PointerMoveEvent e) {
    if (!_pointers.containsKey(e.pointer)) return;
    _pointers[e.pointer] = e.position;
    if (_cornerDrag) {
      // Growing away from the corner it rests in: the box follows the
      // pointer along the axis it has moved further on.
      final by = e.position - _down;
      final x = by.dx * (_anchor.x < 0 ? 1 : -1);
      final y = by.dy * (_anchor.y < 0 ? 1 : -1) * _shape;
      final along = x.abs() > y.abs() ? x : y;
      _resizeTo(_cornerWidth + along);
      return;
    }
    if (_pinching) {
      if (_pointers.length == 2) {
        final points = _pointers.values.toList();
        final distance = (points[0] - points[1]).distance;
        _resizeTo(_pinchWidth * distance / _pinchDistance);
      }
      return;
    }
    if (_held || _pointers.length != 1) return;
    _tracker?.addPosition(e.timeStamp, e.position);
    if (!_dragging) {
      if ((e.position - _down).distance <= kTouchSlop) return;
      setState(() => _dragging = true);
      _timer?.cancel();
      _free = true;
      _boundX = _boundY = null;
      // It follows from where the slop was passed, so it does not jump.
      _origin = e.position;
    }
    _moveTo(_startAt + e.position - _origin);
  }

  void _moveTo(Offset at) {
    final area = widget.area;
    // It may be pulled most of the way off a side, to be tucked there, and
    // as far out as it rests tucked, which for a wide one is further.
    final least = math.min(-_size.width * 0.75, _tuckedX(LabPipSide.left));
    final most = math.max(
      area.width - _size.width * 0.25,
      _tuckedX(LabPipSide.right),
    );
    _sx.value = at.dx.clamp(least, math.max(least, most));
    _sy.value = at.dy.clamp(0.0, math.max(0.0, area.height - _size.height));
    // Known before the next frame, for a release that comes first.
    _shown = Offset(_sx.value, _sy.value);
  }

  // Whether [local] is in the corner away from the one it rests in, where a
  // mouse takes hold to resize it.
  bool _inFreeCorner(Offset local) {
    if (_stashed || _free) return false;
    final corner = widget.layout.corner;
    return (corner.x < 0
            ? local.dx > _size.width - _pipCornerGrip
            : local.dx < _pipCornerGrip) &&
        (corner.y < 0
            ? local.dy > _size.height - _pipCornerGrip
            : local.dy < _pipCornerGrip);
  }

  void _onHover(PointerHoverEvent e) {
    final over =
        e.kind == PointerDeviceKind.mouse && _inFreeCorner(e.localPosition);
    if (over != _cornerHover) setState(() => _cornerHover = over);
  }

  void _onExit(PointerExitEvent e) {
    if (_cornerHover) setState(() => _cornerHover = false);
  }

  void _end(PointerEvent e) {
    if (_pointers.remove(e.pointer) == null) return;
    _timer?.cancel();
    if (_cornerDrag) {
      _cornerDrag = false;
      _endResize();
      return;
    }
    if (_pinching) {
      if (_pointers.isEmpty) {
        _pinching = false;
        _endResize();
      }
      return;
    }
    final held = _held;
    if (held) {
      _held = false;
      widget.onHold(false);
    }
    if (_pointers.isNotEmpty) return;
    _tracker?.addPosition(e.timeStamp, e.position);
    final velocity =
        _tracker?.getVelocity().clampMagnitude(0, kMaxFlingVelocity) ??
        Velocity.zero;
    _tracker = null;
    if (_dragging) {
      setState(() => _dragging = false);
      _release(velocity.pixelsPerSecond);
    } else if (e is PointerUpEvent && !held && !_free) {
      _tap(e);
    } else if (_free) {
      // A press that stopped it settling, let go without moving it.
      _settleTo(widget.layout, _shown, Offset.zero);
    }
  }

  void _tap(PointerUpEvent e) {
    if (_stashed) {
      _show();
      return;
    }
    final last = _lastTap;
    _tapTimer?.cancel();
    if (last != null && (e.position - last).distance < kDoubleTapSlop) {
      _lastTap = null;
      _toggleWidth();
    } else {
      _lastTap = e.position;
      _tapTimer = Timer(kDoubleTapTimeout, () => _lastTap = null);
    }
  }

  // Out of its side, to the corner nearest the tab.
  void _show() {
    final layout = widget.layout;
    final middle = _shown + Offset(_size.width / 2, _size.height / 2);
    final corner = Alignment(
      layout.stash == LabPipSide.left ? -1 : 1,
      _quadrant(middle).y,
    );
    _free = true;
    _settleTo(layout.inCorner(corner), _shown, Offset.zero);
  }

  // Where a release sends it: a corner, or a side to be tucked into.
  void _release(Offset velocity) {
    final area = widget.area;
    final at = _shown;
    final middle = at + Offset(_size.width / 2, _size.height / 2);
    final projected = middle + velocity * _flingHorizon;
    LabPipSide past(double x) => x < 0
        ? LabPipSide.left
        : x > area.width
        ? LabPipSide.right
        : LabPipSide.none;
    final here = past(middle.dx);
    final there = past(projected.dx);
    final beyond = there == LabPipSide.left
        ? -projected.dx
        : projected.dx - area.width;
    final sideways = velocity.dx.abs() > velocity.dy.abs() * _sidewaysRatio;
    var side = LabPipSide.none;
    if (velocity.distance > kMinFlingVelocity &&
        sideways &&
        there != LabPipSide.none &&
        beyond > _size.width) {
      // Thrown well past a side.
      side = there;
    } else if (here != LabPipSide.none && here == there) {
      // Left past a side, and not on its way back in.
      side = here;
    }
    final layout = widget.layout;
    if (side != LabPipSide.none) {
      final (top, bottom) = _vertical(_size);
      final y = at.dy.clamp(top, bottom);
      final part = bottom > top ? (y - top) / (bottom - top) : 0.5;
      _settleTo(layout.tucked(side, part), at, Offset(velocity.dx, 0));
    } else {
      _settleTo(layout.inCorner(_quadrant(projected)), at, velocity);
    }
  }

  // Springs from [from] to where [target] puts it, and says so.
  void _settleTo(LabPipLayout target, Offset from, Offset velocity) {
    final to = _rest(target, _size);
    _starting = true;
    _free = true;
    _boundX = (from.dx, to.dx);
    _boundY = (from.dy, to.dy);
    _sx.value = from.dx;
    _sy.value = from.dy;
    _sx.animateWith(
      SpringSimulation(
        _settleSpring,
        from.dx,
        to.dx,
        velocity.dx,
        snapToEnd: true,
        tolerance: _settleTolerance,
      ),
    );
    _sy.animateWith(
      SpringSimulation(
        _settleSpring,
        from.dy,
        to.dy,
        velocity.dy,
        snapToEnd: true,
        tolerance: _settleTolerance,
      ),
    );
    _starting = false;
    if (target != widget.layout) widget.onLayout(target);
    setState(() {});
  }

  // The most it can be: as big as the overlay holds inside its margins, as
  // a phone's picture-in-picture can be, nearly the whole width upright and
  // the whole height on its side. A double tap brings it back.
  double _most(double aspect) {
    final area = widget.area;
    final insets = widget.insets;
    final room = math.min(
      area.width - 2 * labPipMargin - insets.horizontal,
      (area.height - 2 * labPipMargin - insets.vertical) * aspect,
    );
    return math.max(1.0, room);
  }

  void _toggleWidth() {
    final theme = FluentTheme.of(context);
    final atMost = _size.width >= _maxWidth - 0.5;
    _zoomFrom = _size.width;
    _zoom.duration = theme.mediumAnimationDuration;
    _zoom.forward(from: 0);
    widget.onLayout(widget.layout.withWidth(atMost ? null : _maxWidth));
  }

  // A resize begins from the corner it rests in, or from the corner of its
  // box that is nearest when it is being held or is settling.
  void _beginResize() {
    _zoom.stop();
    if (_free) {
      final from = _shown & _size;
      _resizeFrom = from;
      _anchor = _quadrant(from.center);
    } else {
      _resizeFrom = null;
      _anchor = widget.layout.corner;
    }
    _liveWidth ??= _size.width;
  }

  void _beginPinch() {
    _timer?.cancel();
    final points = _pointers.values.toList();
    _pinching = true;
    _dragging = false;
    _pinchDistance = math.max(1, (points[0] - points[1]).distance);
    _pinchWidth = _size.width;
    _sx.stop();
    _sy.stop();
    _beginResize();
  }

  void _resizeTo(double width) {
    setState(() => _liveWidth = width.clamp(_minWidth, _maxWidth));
  }

  void _endResize() {
    final width = _liveWidth;
    final from = _resizeFrom;
    final anchor = _anchor;
    _liveWidth = null;
    _resizeFrom = null;
    _wheelTimer?.cancel();
    if (width == null) return;
    final layout = widget.layout.withWidth(width);
    if (from == null) {
      widget.onLayout(layout);
      setState(() {});
      return;
    }
    // Let go away from its corner: settle into the one it was scaled from.
    _free = true;
    _size = Size(width, width * _size.height / math.max(1, _size.width));
    final at = Offset(
      anchor.x < 0 ? from.left : from.right - width,
      anchor.y < 0 ? from.top : from.bottom - _size.height,
    );
    _settleTo(layout.inCorner(anchor), at, Offset.zero);
  }

  void _onSignal(PointerSignalEvent e) {
    if (_stashed || _free || _pointers.isNotEmpty) return;
    double? factor;
    if (e is PointerScrollEvent &&
        (HardwareKeyboard.instance.isControlPressed ||
            HardwareKeyboard.instance.isMetaPressed)) {
      factor = math.exp(-e.scrollDelta.dy * _wheelZoomRate);
    } else if (e is PointerScaleEvent) {
      factor = e.scale;
    }
    if (factor == null) return;
    final by = factor;
    // Only this takes the wheel, not the pages under it.
    GestureBinding.instance.pointerSignalResolver.register(e, (_) {
      if (_liveWidth == null) _beginResize();
      _resizeTo((_liveWidth ?? _size.width) * by);
      _wheelTimer?.cancel();
      _wheelTimer = Timer(_wheelIdle, _endResize);
    });
  }

  void _onTrackpadStart(ScaleStartDetails d) {
    if (_pointers.isNotEmpty) return;
    _trackpadOn = true;
    _trackpad = _Trackpad.none;
    _trackpadPan = Offset.zero;
  }

  void _onTrackpadUpdate(ScaleUpdateDetails d) {
    if (!_trackpadOn) return;
    if (_trackpad == _Trackpad.none) {
      // A slide has no scale; a pinch has.
      if ((d.scale - 1).abs() > 0.001) {
        if (_stashed) {
          _trackpadOn = false;
          return;
        }
        _trackpad = _Trackpad.zoom;
        _pinchWidth = _size.width;
        _wheelTimer?.cancel();
        _beginResize();
      } else {
        _trackpad = _Trackpad.pan;
        _timer?.cancel();
        _sx.stop();
        _sy.stop();
        _startAt = _shown;
        _free = true;
        _boundX = _boundY = null;
        setState(() => _dragging = true);
      }
    }
    if (_trackpad == _Trackpad.zoom) {
      _resizeTo(_pinchWidth * d.scale);
    } else {
      _trackpadPan += d.focalPointDelta;
      _moveTo(_startAt + _trackpadPan);
    }
  }

  void _onTrackpadEnd(ScaleEndDetails d) {
    if (!_trackpadOn) return;
    _trackpadOn = false;
    if (_trackpad == _Trackpad.zoom) {
      _endResize();
    } else if (_trackpad == _Trackpad.pan) {
      setState(() => _dragging = false);
      _release(d.velocity.pixelsPerSecond);
    }
    _trackpad = _Trackpad.none;
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final area = widget.area;
    final layout = widget.layout;
    final shorter = math.min(area.width, area.height);
    _viewPadding = MediaQuery.viewPaddingOf(context);
    return ListenableBuilder(
      listenable: widget.controller.textureAspect,
      builder: (context, _) {
        final aspect = widget.controller.textureAspect.value;
        final shape = aspect > 0 ? aspect : 16 / 9;
        _shape = shape;
        _maxWidth = _most(shape);
        _minWidth = math.min(_pipMinWidth, _maxWidth);
        double fit(double width) => width.clamp(_minWidth, _maxWidth);
        final standard = fit(
          (shorter * _pipWidthFraction).clamp(_pipMinWidth, _pipMaxWidth),
        );
        return AnimatedBuilder(
          animation: Listenable.merge([_sx, _sy, _zoom]),
          builder: (context, _) {
            var width = fit(layout.width ?? standard);
            final live = _liveWidth;
            if (live != null) {
              width = fit(live);
            } else if (_zoom.isAnimating) {
              width = fit(
                lerp(
                  _zoomFrom,
                  width,
                  theme.animationCurve.transform(_zoom.value),
                ),
              );
            }
            _size = Size(width, width / shape);
            final from = _resizeFrom;
            if (from != null) {
              _shown = Offset(
                _anchor.x < 0 ? from.left : from.right - _size.width,
                _anchor.y < 0 ? from.top : from.bottom - _size.height,
              );
            } else if (_free) {
              _shown = Offset(
                _bounded(_sx.value, _boundX),
                _bounded(_sy.value, _boundY),
              );
            } else {
              _shown = _rest(layout, _size);
            }
            return Positioned(
              left: _shown.dx,
              top: _shown.dy,
              width: _size.width,
              height: _size.height,
              child: _body(context),
            );
          },
        );
      },
    );
  }

  static double lerp(double a, double b, double t) => a + (b - a) * t;

  // Never past where the settle is going, even when the spring would be.
  double _bounded(double value, (double, double)? bound) {
    if (bound == null) return value;
    final (from, to) = bound;
    return from <= to ? math.min(value, to) : math.max(value, to);
  }

  Widget _body(BuildContext context) {
    final side = widget.layout.stash;
    final stashed = side != LabPipSide.none;
    return Semantics(
      container: true,
      label: stashed ? 'Show Scene Picture' : 'Scene Picture',
      button: stashed,
      onTap: stashed ? _show : null,
      child: MouseRegion(
        cursor: _dragging
            ? SystemMouseCursors.grabbing
            : stashed
            ? SystemMouseCursors.click
            : _cornerDrag || _cornerHover
            // The diagonal the free corner is on.
            ? (widget.layout.corner.x == widget.layout.corner.y
                  ? SystemMouseCursors.resizeUpLeftDownRight
                  : SystemMouseCursors.resizeUpRightDownLeft)
            : SystemMouseCursors.grab,
        onHover: _onHover,
        onExit: _onExit,
        child: RawGestureDetector(
          behavior: HitTestBehavior.opaque,
          gestures: {
            _TrackpadRecognizer:
                GestureRecognizerFactoryWithHandlers<_TrackpadRecognizer>(
                  _TrackpadRecognizer.new,
                  (recognizer) => recognizer
                    ..onStart = _onTrackpadStart
                    ..onUpdate = _onTrackpadUpdate
                    ..onEnd = _onTrackpadEnd,
                ),
          },
          child: Listener(
            behavior: HitTestBehavior.opaque,
            onPointerDown: _onDown,
            onPointerMove: _onMove,
            onPointerUp: _end,
            onPointerCancel: _end,
            onPointerSignal: _onSignal,
            child: Stack(
              fit: StackFit.expand,
              children: [
                DecoratedBox(
                  decoration: BoxDecoration(
                    borderRadius: BorderRadius.circular(4),
                    boxShadow: axShadow(_pipElevation),
                  ),
                  child: Card(
                    padding: EdgeInsets.zero,
                    child: LabScenePreview(
                      controller: widget.controller,
                      peeking: false,
                    ),
                  ),
                ),
                if (stashed)
                  Positioned(
                    top: 0,
                    bottom: 0,
                    left: side == LabPipSide.right ? 0 : null,
                    right: side == LabPipSide.left ? 0 : null,
                    width: _pipTab,
                    child: ExcludeSemantics(
                      child: IgnorePointer(
                        child: Center(
                          child: Icon(
                            side == LabPipSide.right
                                ? Symbols.chevron_left
                                : Symbols.chevron_right,
                            size: 20,
                          ),
                        ),
                      ),
                    ),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
