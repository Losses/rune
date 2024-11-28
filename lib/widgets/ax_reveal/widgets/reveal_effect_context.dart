import 'package:flutter/gestures.dart';
import 'package:flutter/scheduler.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../utils/animation_unit.dart';

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

  static RevealEffectContextState? maybeOf(BuildContext context) {
    return context.findAncestorStateOfType<RevealEffectContextState>();
  }
}

class RevealEffectContextState extends State<RevealEffectContext>
    with SingleTickerProviderStateMixin {
  Ticker? _ticker;
  final GlobalKey _containerKey = GlobalKey();

  void _handleTick(_) {
    _currentFrame++;
    _updateAnimations(_currentFrame);

    for (final unit in _units) {
      if (unit.cleanedUpForAnimation) {
        stopAnimation(unit);
        unit.cleanedUpForAnimation = false;
      } else {
        final isPlayingAnimation = unit.mousePressed || unit.mouseReleased;
        if (_mouseInBoundary || isPlayingAnimation) {
          unit.controller.notify();
        }
      }
    }

    _paintedPosition = _currentPosition;

    if (!hasActiveAnimations) {
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

    _ticker = createTicker(_handleTick);
  }

  final List<AnimationUnit> _animationQueue = [];
  final List<AnimationUnit> _units = [];
  int _currentFrame = 0;
  bool _mouseInBoundary = false;
  Offset? _currentPosition;
  Offset? _paintedPosition;

  int get currentFrame => _currentFrame;
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
    if (_animationQueue.isEmpty) {
      _ticker?.stop();
    }
  }

  void _ensureTickerStarted() {
    if (_ticker != null && !_ticker!.isTicking) {
      _ticker!.start();
    }
  }

  bool _isUpdateScheduled = false;

  void _scheduleUpdate() {
    if (!_isUpdateScheduled) {
      _isUpdateScheduled = true;
      SchedulerBinding.instance.addPostFrameCallback((_) {
        _executeUpdate();
        _isUpdateScheduled = false;
      });
    }
  }

  void _executeUpdate() {
    for (final unit in _units) {
      unit.controller.updateMousePosition(_currentPosition);
    }

    if (_mouseInBoundary || hasActiveAnimations) {
      _ensureTickerStarted();
    }
  }

  void forceRefresh() {
    _scheduleUpdate();
  }

  void updateMousePosition(Offset? position) {
    _currentPosition = position;
    _mouseInBoundary = position != null;

    _scheduleUpdate();
  }

  void _updateAnimations(int frame) {
    for (final unit in _animationQueue) {
      if (frame == 0 || unit.currentFrame == frame) {
        continue;
      }

      unit.currentFrame = frame;

      unit.mouseDownAnimateStartFrame ??= frame;

      final relativeFrame = frame - unit.mouseDownAnimateStartFrame!;
      unit.mouseDownAnimateCurrentFrame = relativeFrame;

      double unitLogicFrame = relativeFrame.toDouble();
      if (unit.mouseReleased && unit.mouseDownAnimateReleasedFrame != null) {
        unitLogicFrame += (frame - unit.mouseDownAnimateReleasedFrame!) *
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
      child: Listener(
        onPointerMove: (event) {
          updateMousePosition(event.position);
        },
        onPointerSignal: (event) {
          if (event is PointerScrollEvent || event is PointerScaleEvent) {
            updateMousePosition(event.position);
          }
        },
        child: Container(
          key: _containerKey,
          child: widget.child,
        ),
      ),
    );
  }
}
