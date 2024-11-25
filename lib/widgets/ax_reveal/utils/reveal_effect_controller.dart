import 'package:flutter/scheduler.dart';
import 'package:fluent_ui/fluent_ui.dart';

import 'animation_unit.dart';

class RevealEffectContext extends StatefulWidget {
  final Widget child;

  const RevealEffectContext({
    super.key,
    required this.child,
  });

  @override
  RevealEffectContextState createState() => RevealEffectContextState();

  static RevealEffectContextState of(BuildContext context) {
    return context.findAncestorStateOfType<RevealEffectContextState>()!;
  }
}

class RevealEffectContextState extends State<RevealEffectContext>
    with TickerProviderStateMixin {
  Ticker? _ticker;
  bool _isInitialized = false;
  final GlobalKey _containerKey = GlobalKey();

  void initialize(TickerProvider vsync) {
    if (_isInitialized) return;

    _ticker = vsync.createTicker(_handleTick);
    _ticker?.dispose();
  }

  void _handleTick(Duration elapsed) {
    _currentFrame++;
    _updateAnimations(_currentFrame);

    for (final unit in _units) {
      if (unit.cleanedUpForAnimation) {
        stopAnimation(unit);
        unit.cleanedUpForAnimation = false;
      } else {
        final isPlayingAnimation = _animationQueue.contains(unit);
        if (_mouseInBoundary || isPlayingAnimation) {
          unit.controller.notify();
        }
      }
    }

    _paintedPosition = _currentPosition;

    if (!_mouseInBoundary && !hasActiveAnimations) {
      _ticker?.stop();
    }
  }

  @override
  void dispose() {
    _ticker?.dispose();
    _ticker = null;

    for (final unit in _units) {
      unit.controller.dispose();
    }

    super.dispose();
  }

  @override
  initState() {
    super.initState();
    _isInitialized = true;
  }

  final List<AnimationUnit> _animationQueue = [];
  final List<AnimationUnit> _units = [];
  int _currentFrame = 0;
  bool _mouseInBoundary = false;
  Offset? _currentPosition;
  Offset? _paintedPosition;

  bool get hasActiveAnimations => _animationQueue.isNotEmpty;
  bool get mouseInBoundary => _mouseInBoundary;
  Offset? get currentPosition => _currentPosition;
  Offset? get paintedPosition => _paintedPosition;

  void addUnit(AnimationUnit unit) {
    _units.add(unit);
  }

  void removeUnit(AnimationUnit unit) {
    _units.remove(unit);
    _animationQueue.remove(unit);
  }

  void startAnimation(AnimationUnit unit) {
    if (!_animationQueue.contains(unit)) {
      _animationQueue.add(unit);
      _ensureTickerStarted();
    }
  }

  void stopAnimation(AnimationUnit unit) {
    unit.reset();
    _animationQueue.remove(unit);
    if (_animationQueue.isEmpty && !_mouseInBoundary) {
      _ticker?.stop();
    }
  }

  void _ensureTickerStarted() {
    if (_ticker != null && !_ticker!.isTicking) {
      _ticker!.start();
    }
  }

  void updateMousePosition(Offset? position) {
    _currentPosition = position;
    _mouseInBoundary = position != null;

    // Notify all controllers about the mouse position change
    for (final unit in _units) {
      unit.controller.updateMousePosition(position);
    }

    if (_mouseInBoundary || hasActiveAnimations) {
      _ensureTickerStarted();
    }
  }

  void _updateAnimations(int frame) {
    for (final unit in _animationQueue) {
      if (frame == 0 || unit.currentFrameId == frame) {
        continue;
      }

      unit.currentFrameId = frame;

      unit.mouseDownAnimateStartFrame ??= frame;

      final relativeFrame = frame - unit.mouseDownAnimateStartFrame!;
      unit.mouseDownAnimateCurrentFrame = relativeFrame;

      double unitLogicFrame = relativeFrame.toDouble();
      if (unit.mouseReleased && unit.mouseDownAnimateReleasedFrame != null) {
        unitLogicFrame +=
            (relativeFrame - unit.mouseDownAnimateReleasedFrame!) *
                unit.releaseAnimationAccelerateRate;
      }
      unit.mouseDownAnimateLogicFrame =
          unitLogicFrame / unit.pressAnimationSpeed;

      if (unit.mouseDownAnimateLogicFrame > 1) {
        unit.cleanedUpForAnimation = true;
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    return MouseRegion(
      onHover: (event) {
        updateMousePosition(event.position);
      },
      onExit: (event) {
        updateMousePosition(null);
      },
      child: Container(
        key: _containerKey,
        child: widget.child,
      ),
    );
  }
}

class RevealEffectController extends ChangeNotifier {
  late final AnimationUnit _animationUnit;
  late final RevealEffectContextState _effectContext;
  final GlobalKey _widgetKey = GlobalKey();
  Offset? _localPosition;

  AnimationUnit get animationUnit => _animationUnit;
  bool get mouseReleased => _animationUnit.mouseReleased;
  double get mouseDownAnimateLogicFrame =>
      _animationUnit.mouseDownAnimateLogicFrame;
  Offset? get localPosition => _localPosition;

  RevealEffectController(BuildContext context) {
    _effectContext = RevealEffectContext.of(context);

    _animationUnit = AnimationUnit(controller: this);
    _effectContext.addUnit(_animationUnit);
  }

  void updateMousePosition(Offset? position) {
    if (position != null && _widgetKey.currentContext != null) {
      final RenderBox renderBox =
          _widgetKey.currentContext!.findRenderObject() as RenderBox;
      final Offset localPosition = renderBox.globalToLocal(position);

      _localPosition = localPosition;
    } else {
      _localPosition = null;
    }

    notify();
  }

  void mouseDown() {
    _animationUnit.mouseReleased = false;
    _effectContext.startAnimation(_animationUnit);
  }

  void mouseUp() {
    _animationUnit.mouseReleased = true;
    _animationUnit.mouseDownAnimateReleasedFrame =
        _animationUnit.mouseDownAnimateCurrentFrame;
  }

  void mouseExit() {
    _localPosition = null;
    notify();
  }

  @override
  dispose() {
    super.dispose();
    _effectContext.removeUnit(_animationUnit);
  }

  notify() {
    notifyListeners();
  }

  GlobalKey get widgetKey => _widgetKey;
}
