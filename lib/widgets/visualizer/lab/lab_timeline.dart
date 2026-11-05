import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/physics.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/router/router_aware_flyout_controller.dart';
import 'lab_data.dart';
import 'lab_display_button.dart';
import 'lab_display.dart';
import 'lab_draw_style.dart';
import 'lab_filter.dart';
import 'lab_menu.dart';
import 'lab_rows.dart';
import 'lab_rows_painter.dart';
import 'lab_segmented.dart';
import 'lab_timeline_math.dart';
import 'lab_tooltip.dart';
import 'lab_views.dart';

// Two presses on the edge this close are a double press.
const _doublePress = Duration(milliseconds: 350);

// How far two fingers slide on a trackpad before the slide takes the axis it
// went furthest along, and keeps it to the end.
const _slideSlop = 4.0;

// A slide let go faster than this, in pixels a second, glides on, slowing as
// a scroll view on macOS does.
const _glideMinSpeed = 50.0;
const _glideMaxSpeed = 8000.0;
const _glideDrag = 0.135;

// How close a finger must land to the playhead's head in the ruler to drag
// it, and to the label column's edge to resize the column, in pixels. A finger
// is less exact than a mouse. Below the ruler the playhead is no handle: with
// Follow on it sits in the plot, where a slide pans.
const _touchPlayhead = 20.0;
const _touchEdge = 12.0;

// What one or two fingers on the plot are doing.
enum _Touch {
  // None are down, or the ones that are down are not doing anything.
  idle,
  // One has landed and has not yet moved past the slop or been held.
  pending,
  // One slides the song or the rows along the axis it took.
  pan,
  // One drags the playhead.
  scrub,
  // One drags the label column's edge.
  resize,
  // One was held on the plot: the label follows it.
  tip,
  // Two pinch and slide the song.
  pinch,
}

// The glide after a slide: along [axis], how it slows, when it began on the
// frames' clock (from the first frame after the fingers lift) and how far it
// has gone.
class _Glide {
  _Glide(this.axis, this.motion);

  final Axis axis;
  final FrictionSimulation motion;
  double? start;
  double went = 0;
}

/// The timeline of the lab: a header with the zoom, the display and the rows,
/// and under it the rows of plan and actual strips against the song's time.
///
/// A tap seeks ([onSeek]); a drag scrubs ([onScrub] as it moves, then
/// [onSeek] where it ends). The widget shows [LabTimelineData.songTime],
/// running on from one reading to the next while the song plays, and keeps
/// its own view of the song.
class LabTimeline extends StatefulWidget {
  const LabTimeline({
    super.key,
    required this.data,
    required this.rows,
    required this.onSeek,
    this.onScrub,
    this.labelWidth,
    this.onLabelWidth,
  });

  final LabTimelineData data;

  /// The rows to show and the views of them; the timeline follows it.
  final LabRowsModel rows;

  /// Seeks to a time in seconds: where a tap is, or where a drag ends.
  final ValueChanged<double> onSeek;

  /// A drag is at a time in seconds.
  final ValueChanged<double>? onScrub;

  /// The label column's width as the user set it; none for the default.
  final double? labelWidth;

  /// The label column is resized: [save] is false while the drag goes on, and
  /// true where it ends. A null width is the default.
  final void Function(double? width, {required bool save})? onLabelWidth;

  @override
  State<LabTimeline> createState() => _LabTimelineState();
}

class _LabTimelineState extends State<LabTimeline>
    with SingleTickerProviderStateMixin {
  final _text = LabTextCache();
  late LabTextColors _colors;
  final _portal = OverlayPortalController();
  final _menu = RouterAwareFlyoutController();

  // The part of the song on show, and whether it follows the song.
  LabView _view = LabView.whole(0);
  double _viewLength = 0;
  int? _viewSong;
  bool _follow = true;

  // How far the rows under the pinned ones are scrolled: the position of a
  // scrollable that only keeps it, for the app's scroll bar to show and to
  // drag. The wheel and the trackpad move it from here.
  final _rowsScroll = ScrollController();

  // The pointer over the plot, in the plot's coordinates and in the overlay's.
  // The pointer moving rebuilds only what listens to it: the cursor's layer,
  // the mouse cursor and the label, not the timeline.
  final _hover = ValueNotifier<Offset?>(null);
  Offset _hoverGlobal = Offset.zero;

  // A press in the plot: where it began, and whether it became a drag.
  int? _pressPointer;
  double _pressX = 0;
  bool _dragged = false;
  double? _scrubTime;

  // A drag of the label column's edge: the pointer, where it began and the
  // width then. The edge is hot while it is hovered or dragged.
  int? _resizePointer;
  double _resizeX = 0;
  double _resizeFrom = 0;
  bool _edgeHot = false;
  Duration? _lastEdgeDown;

  // Fingers (and styluses) on the plot, where each is, and what they do.
  final _touches = <int, Offset>{};
  _Touch _touch = _Touch.idle;
  int? _touchPointer;
  Offset _touchStart = Offset.zero;
  Offset _touchStartGlobal = Offset.zero;
  Offset _touchLast = Offset.zero;
  Axis? _touchAxis;
  VelocityTracker? _touchSpeed;
  Timer? _touchHold;
  // Two fingers: how far apart they are, where between them, and how far the
  // focal point has gone sideways before it counts as a slide.
  double _pinchDistance = 0;
  Offset _pinchFocal = Offset.zero;
  double _pinchTravel = 0;
  bool _pinchSliding = false;

  double _lastScale = 1;

  // A two-finger slide or pinch on a trackpad: whether it pinched, the axis
  // it slides along once it is known, how far it went before that, and its
  // speed.
  bool _pinched = false;
  Axis? _slideAxis;
  Offset _slideTravel = Offset.zero;
  Offset _slidePan = Offset.zero;
  VelocityTracker? _slideSpeed;
  _Glide? _glide;

  // The song time drawn at this frame, from the readings (see
  // [LabPlayClock]) on the frames' clock, and the last reading taken in.
  final _clock = LabPlayClock();
  double _lastFrame = 0;
  late final Ticker _ticker = createTicker(_onTick);
  double _shown = 0;
  // Ticks at every frame that moves the playhead, for the plot alone: the
  // toolbar is not built again 60 times a second.
  final _frame = ValueNotifier<int>(0);
  double? _readTime;
  bool _readPlaying = false;
  int? _readSong;

  LabRowsLayout _layout = LabRowsLayout.empty;
  List<LabColumn>? _layoutColumns;
  LabRowSelection? _layoutSelection;
  LabDisplay? _layoutDisplay;
  Set<String>? _layoutEmpty;

  Size _size = Size.zero;

  LabTimelineData get _data => widget.data;
  // An empty song still has a span to draw the frame on.
  double get _length => math.max(_data.length, 1.0);
  double get _now => _scrubTime ?? _shown;

  // The time of the frame being made, in seconds, or out of a frame the last
  // one's: the frames come at a steady pace, so the playhead moves evenly.
  double get _frameSeconds {
    final binding = SchedulerBinding.instance;
    if (binding.schedulerPhase != SchedulerPhase.idle) {
      _lastFrame = binding.currentFrameTimeStamp.inMicroseconds / 1e6;
    }
    return _lastFrame;
  }

  @override
  void initState() {
    super.initState();
    widget.rows.addListener(_onRows);
    // The rows are drawn again as they scroll, wherever from.
    _rowsScroll.addListener(_onRowsScrolled);
    _read();
  }

  void _onRowsScrolled() => _frame.value++;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    // The rows are written as the app writes: its font, its text's color,
    // and its subtitles' for what is said of them.
    final theme = FluentTheme.of(context);
    _text.base = theme.typography.caption ?? const TextStyle();
    _colors = LabTextColors(
      primary: theme.resources.textFillColorPrimary,
      secondary: theme.inactiveColor.withAlpha(160),
    );
  }

  @override
  void didUpdateWidget(LabTimeline old) {
    super.didUpdateWidget(old);
    if (old.rows != widget.rows) {
      old.rows.removeListener(_onRows);
      widget.rows.addListener(_onRows);
    }
    if (!identical(old.data, widget.data)) _read();
  }

  // Takes in a new reading of the song's time, if the data has one.
  void _read() {
    final d = _data;
    if (d.songTime == _readTime &&
        d.playing == _readPlaying &&
        d.song == _readSong) {
      return;
    }
    final another = _readSong != null && d.song != _readSong;
    if (another) _glide = null;
    _clock.read(
      d.songTime,
      playing: d.playing,
      wall: _frameSeconds,
      jump: another,
    );
    _readTime = d.songTime;
    _readPlaying = d.playing;
    _readSong = d.song;
    _shown = _clampTime(_clock.timeAt(_frameSeconds));
    _syncTicker();
  }

  // The frames run while the song plays or a slide glides.
  void _syncTicker() {
    final want = _data.playing || _glide != null;
    if (want && !_ticker.isActive) {
      _ticker.start();
    } else if (!want && _ticker.isActive) {
      _ticker.stop();
    }
  }

  void _onTick(Duration _) {
    final wall = _frameSeconds;
    if (_data.playing) {
      final t = _clampTime(_clock.timeAt(wall));
      if (t != _shown) {
        _shown = t;
        _frame.value++;
      }
    }
    final glide = _glide;
    if (glide != null) {
      // Its first frame moves it a frame's worth.
      final t = wall - (glide.start ??= wall - 1 / 60);
      final x = glide.motion.x(t);
      final moved = _slide(glide.axis, x - glide.went, _geometry());
      glide.went = x;
      if (!moved || glide.motion.isDone(t)) _glide = null;
    }
    _syncTicker();
  }

  void _onRows() => setState(() {});

  @override
  void dispose() {
    _touchHold?.cancel();
    _ticker.dispose();
    _frame.dispose();
    _rowsScroll.dispose();
    widget.rows.removeListener(_onRows);
    _menu.dispose();
    _hover.dispose();
    _text.dispose();
    super.dispose();
  }

  // The view for the current build: the whole song for another song or when
  // a song arrives, kept inside the song when its length moves a little, and
  // following the playhead when asked.
  LabView _currentView() {
    if (_viewSong != _data.song || _viewLength <= 0 || _data.length <= 0) {
      _view = LabView.whole(_length);
    }
    _viewSong = _data.song;
    _viewLength = _data.length;
    var view = labClampView(_view, _length);
    if (_follow && !view.isWhole(_length)) {
      view = labFollowView(view, _length, _now);
    }
    return _view = view;
  }

  LabGeometry _geometry() => LabGeometry(
    width: _size.width,
    height: _size.height,
    view: _view,
    scrollY: _scrollY,
    length: _length,
    labelWidth: widget.labelWidth,
  );

  // How far the rows are scrolled, inside the rows: after a resize or a
  // filter, before the position has been told the new heights.
  double get _scrollY {
    final rows = math.max(0.0, _size.height - LabGeometry.ruler);
    final most = math.max(0.0, _layout.contentHeight - rows);
    final at = _rowsScroll.hasClients ? _rowsScroll.offset : 0.0;
    return at.clamp(0.0, most);
  }

  // Whether [p] is on the scroll bar: the margin right of the plot, by the
  // rows that scroll, when they do.
  bool _onScrollbar(Offset p, LabGeometry g) =>
      g.maxScroll(_layout.contentHeight) > 0 &&
      p.dx >= g.label + g.plotWidth &&
      p.dy >= LabGeometry.ruler + _layout.fixed(g.rowsHeight).height;

  // Whether x is on the edge between the label column and the plot.
  bool _onEdge(double x, LabGeometry g) => LabCursorPainter.onEdge(x, g);

  void _setFollow(bool on) => setState(() {
    _follow = on;
    if (on) _glide = null;
  });

  void _zoomAround(double center, double factor) {
    if (_length <= 0) return;
    setState(() {
      _view = labZoomView(_view, _length, _follow ? _now : center, factor);
    });
  }

  void _zoomButton(double factor) =>
      _zoomAround(_follow ? _now : (_view.start + _view.end) / 2, factor);

  // Moves the view [seconds] along the song; whether it moved.
  bool _pan(double seconds) {
    if (_view.isWhole(_length)) return false;
    final next = labPanView(_view, _length, seconds);
    if (next.start == _view.start && next.end == _view.end) return false;
    setState(() {
      _view = next;
      _follow = false;
    });
    return true;
  }

  // Scrolls the rows [dy] down; whether they moved.
  bool _scrollRowsBy(double dy, LabGeometry g) {
    if (!_rowsScroll.hasClients) return false;
    final max = g.maxScroll(_layout.contentHeight);
    final next = math.min(math.max(_scrollY + dy, 0.0), math.max(max, 0.0));
    if (next == _scrollY) return false;
    _rowsScroll.jumpTo(next);
    return true;
  }

  // A wheel's turn: the rows scroll, or the song moves when they all fit.
  void _scrollRows(double dy, LabGeometry g) {
    if (g.maxScroll(_layout.contentHeight) > 0) {
      _scrollRowsBy(dy, g);
    } else {
      _pan(dy / g.plotWidth * _view.span);
    }
  }

  // Moves what shows by [delta] pixels along [axis], as fingers on it would:
  // the song sideways, the rows up and down. Whether it moved.
  bool _slide(Axis axis, double delta, LabGeometry g) => axis == Axis.horizontal
      ? _pan(-delta / g.plotWidth * _view.span)
      : _scrollRowsBy(-delta, g);

  double _timeAtOrNow(double x, LabGeometry g) =>
      x >= g.label ? g.timeOf(x) : _now;

  // ---------- pointer ----------

  void _onSignal(PointerSignalEvent e) {
    _glide = null;
    if (e is PointerScrollEvent) {
      GestureBinding.instance.pointerSignalResolver.register(e, _onScroll);
    } else if (e is PointerScaleEvent) {
      GestureBinding.instance.pointerSignalResolver.register(e, (e) {
        e as PointerScaleEvent;
        final g = _geometry();
        _zoomAround(_timeAtOrNow(e.localPosition.dx, g), 1 / e.scale);
      });
    }
  }

  void _onScroll(PointerSignalEvent event) {
    final e = event as PointerScrollEvent;
    final g = _geometry();
    final dx = e.scrollDelta.dx;
    final dy = e.scrollDelta.dy;
    final keys = HardwareKeyboard.instance;
    if (keys.isControlPressed || keys.isMetaPressed) {
      _zoomAround(_timeAtOrNow(e.localPosition.dx, g), math.exp(dy * 0.0025));
    } else if (keys.isShiftPressed || dx.abs() > dy.abs()) {
      final d = keys.isShiftPressed && dx == 0 ? dy : dx;
      _pan(d / g.plotWidth * _view.span);
    } else {
      _scrollRows(dy, g);
    }
  }

  void _onPanZoomStart(PointerPanZoomStartEvent e) {
    _glide = null;
    _lastScale = 1;
    _pinched = false;
    _slideAxis = null;
    _slideTravel = Offset.zero;
    _slidePan = Offset.zero;
    _slideSpeed = VelocityTracker.withKind(PointerDeviceKind.trackpad)
      ..addPosition(e.timeStamp, Offset.zero);
  }

  void _onPanZoomUpdate(PointerPanZoomUpdateEvent e) {
    final g = _geometry();
    if ((e.scale - _lastScale).abs() > 1e-4) {
      _pinched = true;
      _zoomAround(_timeAtOrNow(e.localPosition.dx, g), _lastScale / e.scale);
      _lastScale = e.scale;
      return;
    }
    if (_pinched) return;
    // Two fingers sliding: the content follows them, along one axis.
    _slidePan = e.pan;
    _slideSpeed?.addPosition(e.timeStamp, e.pan);
    var d = e.panDelta;
    var axis = _slideAxis;
    if (axis == null) {
      _slideTravel += d;
      if (_slideTravel.distance < _slideSlop) return;
      axis = _slideAxis = _slideTravel.dx.abs() > _slideTravel.dy.abs()
          ? Axis.horizontal
          : Axis.vertical;
      d = _slideTravel;
    }
    _slide(axis, axis == Axis.horizontal ? d.dx : d.dy, g);
  }

  void _onPanZoomEnd(PointerPanZoomEndEvent e) {
    final axis = _slideAxis;
    final tracker = _slideSpeed;
    _slideAxis = null;
    _slideSpeed = null;
    if (axis == null || tracker == null || _pinched) return;
    // Fingers that stopped before they lifted do not glide.
    tracker.addPosition(e.timeStamp, _slidePan);
    final speed = tracker.getVelocity().pixelsPerSecond;
    _glideOn(axis, axis == Axis.horizontal ? speed.dx : speed.dy);
  }

  // Lets a slide along [axis] go on at [v] pixels a second, if that is fast
  // enough, slowing as it goes.
  void _glideOn(Axis axis, double v) {
    if (v.abs() < _glideMinSpeed) return;
    _glide = _Glide(
      axis,
      FrictionSimulation(
        _glideDrag,
        0,
        v.clamp(-_glideMaxSpeed, _glideMaxSpeed),
        tolerance: const Tolerance(velocity: 20),
      ),
    );
    _syncTicker();
  }

  void _onDown(PointerDownEvent e) {
    _glide = null;
    if (_isTouch(e.kind)) {
      _onTouchDown(e);
      return;
    }
    final g = _geometry();
    // The scroll bar takes it.
    if (_onScrollbar(e.localPosition, g)) return;
    final onEdge = _onEdge(e.localPosition.dx, g);
    // A right click, or a click with Control, on a row's name.
    final menu =
        e.buttons & kSecondaryButton != 0 ||
        e.buttons & kPrimaryButton != 0 &&
            HardwareKeyboard.instance.isControlPressed;
    if (menu && !onEdge && e.localPosition.dx < g.label) {
      _openRowMenu(e.localPosition, e.position, g);
      return;
    }
    if (e.buttons & kPrimaryButton == 0) return;
    if (onEdge) {
      final last = _lastEdgeDown;
      _lastEdgeDown = e.timeStamp;
      if (last != null && e.timeStamp - last <= _doublePress) {
        _lastEdgeDown = null;
        widget.onLabelWidth?.call(null, save: true);
        return;
      }
      _resizePointer = e.pointer;
      _resizeX = e.localPosition.dx;
      _resizeFrom = g.label;
      setState(() => _edgeHot = true);
      return;
    }
    if (e.localPosition.dx < g.label) {
      // A click on the name of a level switches it between its band and a
      // line.
      _toggleLevelAt(e.localPosition, g);
      return;
    }
    _pressPointer = e.pointer;
    _pressX = e.localPosition.dx;
    _dragged = false;
  }

  void _onMove(PointerMoveEvent e) {
    if (_touches.containsKey(e.pointer)) {
      _onTouchMove(e);
      return;
    }
    _setHover(e.localPosition, e.position);
    if (e.pointer == _resizePointer) {
      _resizeTo(e.localPosition.dx);
      return;
    }
    if (e.pointer != _pressPointer) return;
    if ((e.localPosition.dx - _pressX).abs() > 3) _dragged = true;
    if (!_dragged) return;
    _scrubTo(e.localPosition.dx);
  }

  // The label column is dragged to the pointer at [x].
  void _resizeTo(double x) {
    final w = LabGeometry.clampLabel(_resizeFrom + x - _resizeX, _size.width);
    _lastEdgeDown = null;
    widget.onLabelWidth?.call(w, save: false);
  }

  // The playhead is dragged to the pointer at [x].
  void _scrubTo(double x) {
    final t = _clampTime(_geometry().timeOf(x));
    setState(() => _scrubTime = t);
    widget.onScrub?.call(t);
  }

  // A click on the name of a level switches it between its band and a line.
  void _toggleLevelAt(Offset at, LabGeometry g) {
    final column = _expandableAt(at, g);
    if (column == null) return;
    final rows = widget.rows;
    rows.setDisplay(
      rows.display.withExpanded(
        column.id,
        on: !rows.display.isExpanded(column.id),
      ),
    );
  }

  void _endResize({required bool save}) {
    _resizePointer = null;
    setState(() => _edgeHot = false);
    if (save) widget.onLabelWidth?.call(_geometry().label, save: true);
  }

  void _onUp(PointerUpEvent e) {
    if (_touches.containsKey(e.pointer)) {
      _touchLift(e.pointer, e.localPosition, e.timeStamp);
      return;
    }
    if (e.pointer == _resizePointer) {
      // A press that did not move keeps the width, so it is not saved.
      _endResize(save: (e.localPosition.dx - _resizeX).abs() > 0);
      return;
    }
    if (e.pointer != _pressPointer) return;
    _pressPointer = null;
    final t = _dragged && _scrubTime != null
        ? _scrubTime!
        : _clampTime(_geometry().timeOf(e.localPosition.dx));
    _dragged = false;
    setState(() => _scrubTime = null);
    if (_data.length > 0) widget.onSeek(t);
  }

  void _onCancel(PointerCancelEvent e) {
    if (_touches.containsKey(e.pointer)) {
      _touchLift(e.pointer, e.localPosition, e.timeStamp, cancelled: true);
      return;
    }
    if (e.pointer == _resizePointer) {
      _endResize(save: true);
      return;
    }
    if (e.pointer != _pressPointer) return;
    _pressPointer = null;
    _dragged = false;
    setState(() => _scrubTime = null);
  }

  double _clampTime(double t) => math.min(math.max(t, 0), _length);

  void _setHover(Offset local, Offset global) {
    _hoverGlobal = global;
    _hover.value = local;
    if (!_portal.isShowing) _portal.show();
  }

  void _clearHover() {
    if (_hover.value == null) return;
    _hover.value = null;
    if (_portal.isShowing) _portal.hide();
  }

  // The column of a level whose name is at [at], which a click expands to a
  // line or folds back to its band.
  LabColumn? _expandableAt(Offset at, LabGeometry g) {
    if (at.dx >= g.label) return null;
    final viewY = at.dy - LabGeometry.ruler;
    if (viewY < 0) return null;
    final column = _layout.rowAtView(viewY, _scrollY, g.rowsHeight)?.column;
    return column != null && labExpandable(column) ? column : null;
  }

  // ---------- touch ----------

  static bool _isTouch(PointerDeviceKind kind) =>
      kind == PointerDeviceKind.touch ||
      kind == PointerDeviceKind.stylus ||
      kind == PointerDeviceKind.invertedStylus;

  // A finger lands. The first one waits to see what it is: a tap, a hold, a
  // slide, or a drag of the playhead or of the label column's edge. A second
  // one makes a pinch of it and cancels what the first began.
  void _onTouchDown(PointerDownEvent e) {
    _touches[e.pointer] = e.localPosition;
    if (_touches.length > 1) {
      _startPinch();
      return;
    }
    final g = _geometry();
    final at = e.localPosition;
    _touchPointer = e.pointer;
    _touchStart = _touchLast = at;
    _touchStartGlobal = e.position;
    _touchAxis = null;
    _touchSpeed = VelocityTracker.withKind(e.kind)
      ..addPosition(e.timeStamp, Offset.zero);
    if (_onScrollbar(at, g)) {
      // The scroll bar takes it.
      _touch = _Touch.idle;
    } else if ((at.dx - g.label).abs() <= _touchEdge) {
      _touch = _Touch.resize;
      _resizePointer = e.pointer;
      _resizeX = at.dx;
      _resizeFrom = g.label;
      setState(() => _edgeHot = true);
    } else if (at.dx >= g.label &&
        at.dy < LabGeometry.ruler &&
        (at.dx - g.xOf(_now)).abs() <= _touchPlayhead) {
      _touch = _Touch.scrub;
      _pressPointer = e.pointer;
      _pressX = at.dx;
      _dragged = false;
    } else {
      _touch = _Touch.pending;
      _touchHold = Timer(kLongPressTimeout, _onTouchHold);
    }
  }

  // The finger has stayed: on a row's name it opens the row's menu, on the
  // plot the label follows it.
  void _onTouchHold() {
    if (_touch != _Touch.pending || !mounted) return;
    final g = _geometry();
    if (_touchStart.dx < g.label) {
      _touch = _Touch.idle;
      _openRowMenu(_touchStart, _touchStartGlobal, g);
    } else {
      _touch = _Touch.tip;
      _setHover(_touchLast, _touchStartGlobal);
    }
  }

  void _onTouchMove(PointerMoveEvent e) {
    _touches[e.pointer] = e.localPosition;
    if (_touch == _Touch.pinch) {
      _updatePinch();
      return;
    }
    if (e.pointer != _touchPointer) return;
    final at = e.localPosition;
    switch (_touch) {
      case _Touch.resize:
        _resizeTo(at.dx);
      case _Touch.scrub:
        if ((at.dx - _pressX).abs() > 3) _dragged = true;
        if (_dragged) _scrubTo(at.dx);
      case _Touch.tip:
        _touchLast = at;
        _setHover(at, e.position);
      case _Touch.pending:
        _touchSpeed?.addPosition(e.timeStamp, at - _touchStart);
        final travel = at - _touchStart;
        if (travel.distance < kTouchSlop) return;
        // It moved: not a tap or a hold, but a slide along the axis it went
        // furthest on. What it has gone so far moves the content at once.
        _touchHold?.cancel();
        _touch = _Touch.pan;
        final axis = _touchAxis = travel.dx.abs() > travel.dy.abs()
            ? Axis.horizontal
            : Axis.vertical;
        _slide(
          axis,
          axis == Axis.horizontal ? travel.dx : travel.dy,
          _geometry(),
        );
        _touchLast = at;
      case _Touch.pan:
        _touchSpeed?.addPosition(e.timeStamp, at - _touchStart);
        final axis = _touchAxis!;
        final d = at - _touchLast;
        _touchLast = at;
        _slide(axis, axis == Axis.horizontal ? d.dx : d.dy, _geometry());
      case _Touch.idle:
      case _Touch.pinch:
        break;
    }
  }

  // A finger lifts, or is taken away by the system when [cancelled].
  void _touchLift(
    int pointer,
    Offset at,
    Duration time, {
    bool cancelled = false,
  }) {
    _touches.remove(pointer);
    if (_touch == _Touch.pinch) {
      if (_touches.length >= 2) {
        _resetPinch();
      } else {
        _touch = _Touch.idle;
      }
    } else if (pointer == _touchPointer) {
      _touchHold?.cancel();
      final g = _geometry();
      switch (_touch) {
        case _Touch.pending:
          if (!cancelled) {
            if (at.dx < g.label) {
              _toggleLevelAt(at, g);
            } else if (_data.length > 0) {
              widget.onSeek(_clampTime(g.timeOf(at.dx)));
            }
          }
        case _Touch.pan:
          final tracker = _touchSpeed;
          final axis = _touchAxis;
          if (!cancelled && tracker != null && axis != null) {
            // A finger that stopped before it lifted does not glide.
            tracker.addPosition(time, at - _touchStart);
            final speed = tracker.getVelocity().pixelsPerSecond;
            _glideOn(axis, axis == Axis.horizontal ? speed.dx : speed.dy);
          }
        case _Touch.scrub:
          _pressPointer = null;
          final t = _dragged && _scrubTime != null
              ? _scrubTime!
              : _clampTime(g.timeOf(at.dx));
          _dragged = false;
          setState(() => _scrubTime = null);
          // A press on the playhead that did not move seeks, as a tap does.
          if (!cancelled && _data.length > 0) widget.onSeek(t);
        case _Touch.resize:
          _endResize(save: (at.dx - _resizeX).abs() > 0);
        case _Touch.tip:
          _clearHover();
        case _Touch.idle:
        case _Touch.pinch:
          break;
      }
      _touch = _Touch.idle;
    }
    if (_touches.isEmpty) {
      _touchPointer = null;
      _touchSpeed = null;
      _touch = _Touch.idle;
    }
  }

  // A second finger: whatever the first began is dropped without seeking or
  // opening a menu, and the two make a pinch.
  void _startPinch() {
    _touchHold?.cancel();
    switch (_touch) {
      case _Touch.scrub:
        _pressPointer = null;
        _dragged = false;
        setState(() => _scrubTime = null);
      case _Touch.resize:
        _endResize(save: true);
      case _Touch.tip:
        _clearHover();
      case _Touch.idle:
      case _Touch.pending:
      case _Touch.pan:
      case _Touch.pinch:
        break;
    }
    _touch = _Touch.pinch;
    _pinchTravel = 0;
    _pinchSliding = false;
    _resetPinch();
  }

  // Takes the first two fingers' distance and focal point as the ones to
  // move on from.
  void _resetPinch() {
    final two = _touches.values.take(2).toList();
    _pinchDistance = (two[0] - two[1]).distance;
    _pinchFocal = (two[0] + two[1]) / 2;
  }

  // Two fingers moved: the song zooms around the time under the focal point,
  // and slides with it once it has gone sideways past the slop, so that
  // zooming alone leaves Follow on.
  void _updatePinch() {
    final two = _touches.values.take(2).toList();
    final distance = (two[0] - two[1]).distance;
    final focal = (two[0] + two[1]) / 2;
    final g = _geometry();
    if (distance > 0 && _pinchDistance > 0) {
      _zoomAround(_timeAtOrNow(_pinchFocal.dx, g), _pinchDistance / distance);
    }
    _pinchTravel += focal.dx - _pinchFocal.dx;
    if (!_pinchSliding && _pinchTravel.abs() >= kTouchSlop) {
      _pinchSliding = true;
      _pan(-_pinchTravel / g.plotWidth * _view.span);
    } else if (_pinchSliding) {
      _pan(-(focal.dx - _pinchFocal.dx) / g.plotWidth * _view.span);
    }
    _pinchDistance = distance;
    _pinchFocal = focal;
  }

  // ---------- row menu ----------

  void _openRowMenu(Offset local, Offset global, LabGeometry g) {
    final viewY = local.dy - LabGeometry.ruler;
    if (viewY < 0) return;
    final row = _layout.rowAtView(viewY, _scrollY, g.rowsHeight);
    final column = row?.column;
    if (column == null) return;
    _clearHover();
    final rows = widget.rows;
    final hasPlan = column.plan == LabPlanAvailability.yes;
    final hasActual = column.hasActual;
    final related = LabRowSelection.related(column, _data.columns);
    final pinned = rows.selection.isPinned(column.id);

    void change(LabRowSelection Function(LabRowSelection) f) =>
        rows.setSelection(f(rows.selection));

    showLabFlyout<void>(
      context,
      _menu,
      position: global,
      builder: (context) => MenuFlyout(
        items: [
          MenuFlyoutItem(
            leading: const Icon(Symbols.visibility_off),
            text: const Text('Hide Row'),
            onPressed: () => change((s) => s.withVisible(column, on: false)),
          ),
          const MenuFlyoutSeparator(),
          MenuFlyoutItem(
            text: const Text('Plan Only'),
            onPressed: hasPlan
                ? () => change(
                    (s) => s.setPart(column.id, plan: true, actual: false),
                  )
                : null,
          ),
          MenuFlyoutItem(
            text: const Text('Actual Only'),
            onPressed: hasActual
                ? () => change(
                    (s) => s.setPart(column.id, plan: false, actual: true),
                  )
                : null,
          ),
          MenuFlyoutItem(
            text: const Text('Both'),
            onPressed: hasPlan && hasActual
                ? () => change((s) => s.withVisible(column, on: true))
                : null,
          ),
          const MenuFlyoutSeparator(),
          MenuFlyoutItem(
            leading: Icon(pinned ? Symbols.keep_off : Symbols.keep),
            text: Text(pinned ? 'Unpin' : 'Pin to Top'),
            onPressed: () =>
                change((s) => pinned ? s.unpin(column.id) : s.pin(column.id)),
          ),
          if (related.isNotEmpty)
            MenuFlyoutItem(
              leading: const Icon(Symbols.account_tree),
              text: const Text('Show Related Rows'),
              onPressed: () => change((s) {
                var next = s;
                for (final r in related) {
                  next = next.withVisible(r, on: true);
                }
                return next;
              }),
            ),
          MenuFlyoutItem(
            leading: const Icon(Symbols.filter_list),
            text: const Text('Show in Rows…'),
            onPressed: () {
              if (!mounted) return;
              showLabRowsDialog(
                this.context,
                model: rows,
                columns: _data.columns,
                reveal: column.id,
              );
            },
          ),
        ],
      ),
    );
  }

  // ---------- header ----------

  // The name of the view the rows follow, and whether they were changed
  // from it.
  String _viewLabel() {
    final rows = widget.rows;
    final columns = _data.columns;
    final name = rows.currentView(columns)?.name ?? rows.currentName;
    return rows.isEdited(columns) ? '$name (Edited)' : name;
  }

  Widget _header() {
    final span = _view.span;
    final spanLabel = _view.isWhole(_length)
        ? 'Whole Song'
        : '${span < 20 ? span.toStringAsFixed(1) : span.round()} s';

    final left = Wrap(
      spacing: 6,
      runSpacing: 6,
      crossAxisAlignment: WrapCrossAlignment.center,
      children: [
        ToggleButton(
          checked: _follow,
          onChanged: _setFollow,
          child: const Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(Symbols.my_location, size: 16),
              SizedBox(width: 4),
              Text('Follow'),
            ],
          ),
        ),
        // Less, how much of the song shows (a click shows all of it), more.
        LabButtonGroup(
          buttons: [
            LabGroupButton(
              tooltip: 'Zoom Out',
              onPressed: () => _zoomButton(1.8),
              child: const Icon(Symbols.remove),
            ),
            LabGroupButton(
              tooltip: 'Show Whole Song',
              width: 84,
              onPressed: () => setState(() => _view = LabView.whole(_length)),
              child: Text(
                spanLabel,
                maxLines: 1,
                style: const TextStyle(
                  fontFeatures: [FontFeature.tabularFigures()],
                ),
              ),
            ),
            LabGroupButton(
              tooltip: 'Zoom In',
              onPressed: () => _zoomButton(1 / 1.8),
              child: const Icon(Symbols.add),
            ),
          ],
        ),
      ],
    );

    final right = Wrap(
      spacing: 6,
      runSpacing: 6,
      crossAxisAlignment: WrapCrossAlignment.center,
      children: [
        LabDisplayButton(
          model: widget.rows,
          columns: _data.columns,
          empty: _data.emptyColumns,
        ),
        // The rows: the view they follow, which the dialog chooses.
        Tooltip(
          message: 'Rows: ${_viewLabel()}',
          child: Button(
            onPressed: () => showLabRowsDialog(
              context,
              model: widget.rows,
              columns: _data.columns,
            ),
            child: const Icon(Symbols.filter_list, size: 16),
          ),
        ),
      ],
    );

    return Padding(
      padding: const EdgeInsets.all(6),
      child: Wrap(
        alignment: WrapAlignment.spaceBetween,
        crossAxisAlignment: WrapCrossAlignment.center,
        spacing: 10,
        runSpacing: 6,
        children: [left, right],
      ),
    );
  }

  // The first section that starts more than 4 s ahead.
  // ---------- build ----------

  @override
  Widget build(BuildContext context) {
    if (!identical(_layoutColumns, _data.columns) ||
        !identical(_layoutSelection, widget.rows.selection) ||
        !identical(_layoutDisplay, widget.rows.display) ||
        !identical(_layoutEmpty, _data.emptyColumns)) {
      _layoutColumns = _data.columns;
      _layoutSelection = widget.rows.selection;
      _layoutDisplay = widget.rows.display;
      _layoutEmpty = _data.emptyColumns;
      _layout = LabRowsLayout.build(
        _data.columns,
        widget.rows.selection,
        display: widget.rows.display,
        empty: _data.emptyColumns,
      );
    }
    _currentView();

    return Card(
      padding: EdgeInsets.zero,
      child: ClipRRect(
        // Inside the card's border.
        borderRadius: BorderRadius.circular(3),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _header(),
            const Divider(
              style: DividerThemeData(
                horizontalMargin: EdgeInsets.zero,
                verticalMargin: EdgeInsets.zero,
              ),
            ),
            // The plot follows the playhead at every frame; the view it shows
            // moves with it when it follows.
            Expanded(
              child: ValueListenableBuilder<int>(
                valueListenable: _frame,
                builder: (context, _, _) {
                  _currentView();
                  return _body();
                },
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _body() {
    return LayoutBuilder(
      builder: (context, constraints) {
        _size = constraints.biggest;
        final geometry = _geometry();
        final fixed = _layout.fixed(geometry.rowsHeight);

        final plot = OverlayPortal(
          controller: _portal,
          overlayChildBuilder: _tipBuilder,
          child: FlyoutTarget(
            controller: _menu.controller,
            child: Listener(
              behavior: HitTestBehavior.opaque,
              onPointerDown: _onDown,
              onPointerMove: _onMove,
              onPointerUp: _onUp,
              onPointerCancel: _onCancel,
              onPointerSignal: _onSignal,
              onPointerPanZoomStart: _onPanZoomStart,
              onPointerPanZoomUpdate: _onPanZoomUpdate,
              onPointerPanZoomEnd: _onPanZoomEnd,
              child: ValueListenableBuilder<Offset?>(
                valueListenable: _hover,
                // The rows and the cursor's layer are not rebuilt as the
                // pointer moves.
                child: Stack(
                  fit: StackFit.expand,
                  children: [
                    RepaintBoundary(
                      child: CustomPaint(
                        size: constraints.biggest,
                        painter: LabRowsPainter(
                          data: _data,
                          layout: _layout,
                          geometry: geometry,
                          // The values by the names change with the
                          // readings; only the playhead runs every frame.
                          now: _scrubTime ?? _data.songTime,
                          text: _text,
                          colors: _colors,
                        ),
                      ),
                    ),
                    IgnorePointer(
                      child: RepaintBoundary(
                        child: CustomPaint(
                          size: constraints.biggest,
                          painter: LabCursorPainter(
                            geometry: geometry,
                            now: _now,
                            devicePixelRatio: MediaQuery.devicePixelRatioOf(
                              context,
                            ),
                            pointer: _hover,
                            resizing: _edgeHot,
                          ),
                        ),
                      ),
                    ),
                  ],
                ),
                builder: (context, hover, child) => MouseRegion(
                  cursor: _edgeHotAt(hover, geometry)
                      ? SystemMouseCursors.resizeColumn
                      : hover != null && _onScrollbar(hover, geometry)
                      ? SystemMouseCursors.basic
                      : hover != null && _expandableAt(hover, geometry) != null
                      ? SystemMouseCursors.click
                      : SystemMouseCursors.precise,
                  onHover: (e) => _setHover(e.localPosition, e.position),
                  onExit: (_) => _clearHover(),
                  child: child,
                ),
              ),
            ),
          ),
        );

        // The app's scroll bar, by the rows that scroll: under the ruler and
        // the pinned rows, and only while they move or the pointer is on it.
        final bar = ScrollbarTheme.of(context);
        EdgeInsets below(EdgeInsetsGeometry? padding) =>
            (padding?.resolve(TextDirection.ltr) ?? EdgeInsets.zero) +
            EdgeInsets.only(top: LabGeometry.ruler + fixed.height);
        return MediaQuery.removeViewPadding(
          context: context,
          removeLeft: true,
          removeTop: true,
          removeRight: true,
          removeBottom: true,
          child: Scrollbar(
            controller: _rowsScroll,
            thumbVisibility: false,
            style: ScrollbarThemeData(
              padding: below(bar.padding),
              hoveringPadding: below(bar.hoveringPadding),
            ),
            child: ScrollConfiguration(
              // Nothing drags it but the bar: the plot's own handlers take
              // the wheel and the trackpad, so that a slide keeps one axis.
              behavior: ScrollConfiguration.of(
                context,
              ).copyWith(scrollbars: false, dragDevices: const {}),
              child: Scrollable(
                controller: _rowsScroll,
                physics: const ClampingScrollPhysics(),
                viewportBuilder: (context, offset) => _RowsViewport(
                  offset: offset,
                  extent: geometry.rowsHeight - fixed.height,
                  content: _layout.contentHeight - fixed.height,
                  child: plot,
                ),
              ),
            ),
          ),
        );
      },
    );
  }

  // Whether the edge of the label column is hot: it is dragged, or the
  // pointer [hover] is on it.
  bool _edgeHotAt(Offset? hover, LabGeometry g) =>
      _edgeHot || (hover != null && _onEdge(hover.dx, g));

  Widget _tipBuilder(BuildContext overlayContext) =>
      ValueListenableBuilder<Offset?>(
        valueListenable: _hover,
        builder: (context, hover, _) => _tipLabel(hover),
      );

  // The label for what is under the pointer [hover]; none while the pointer
  // is away, a drag goes on or the label column's edge is hot.
  Widget _tipLabel(Offset? hover) {
    final overlay = Overlay.maybeOf(context);
    final box = overlay?.context.findRenderObject() as RenderBox?;
    if (hover == null || box == null || !box.attached) {
      return const SizedBox.shrink();
    }
    final geometry = _geometry();
    if (_dragged || _edgeHotAt(hover, geometry)) {
      return const SizedBox.shrink();
    }
    final tip = labTipAt(
      data: _data,
      layout: _layout,
      geometry: geometry,
      x: hover.dx,
      y: hover.dy,
    );
    if (tip == null) return const SizedBox.shrink();
    return LabTipLabel(tip: tip, anchor: box.globalToLocal(_hoverGlobal));
  }
}

/// The plot as the viewport of the rows' scrollable: it tells the scroll
/// position how tall the rows that scroll are and how much of them shows, and
/// draws them itself where the position says.
class _RowsViewport extends SingleChildRenderObjectWidget {
  const _RowsViewport({
    required this.offset,
    required this.extent,
    required this.content,
    required super.child,
  });

  final ViewportOffset offset;

  /// How much of the rows that scroll shows.
  final double extent;

  /// How tall the rows that scroll are.
  final double content;

  @override
  _RenderRowsViewport createRenderObject(BuildContext context) =>
      _RenderRowsViewport(offset, extent, content);

  @override
  void updateRenderObject(BuildContext context, _RenderRowsViewport box) {
    box
      ..offset = offset
      ..extent = extent
      ..content = content;
  }
}

class _RenderRowsViewport extends RenderProxyBox {
  _RenderRowsViewport(this._offset, this._extent, this._content);

  ViewportOffset _offset;
  set offset(ViewportOffset value) {
    if (identical(value, _offset)) return;
    _offset = value;
    markNeedsLayout();
  }

  double _extent;
  set extent(double value) {
    if (value == _extent) return;
    _extent = value;
    markNeedsLayout();
  }

  double _content;
  set content(double value) {
    if (value == _content) return;
    _content = value;
    markNeedsLayout();
  }

  @override
  void performLayout() {
    super.performLayout();
    final extent = math.max(0.0, _extent);
    _offset.applyViewportDimension(extent);
    // A position past the new end stays as it is: the scroll physics
    // (clamping) bring it back, and what is drawn is held at the end by
    // [_scrollY] meanwhile. The plot was built at the end already.
    _offset.applyContentDimensions(0, math.max(0.0, _content - extent));
  }
}
