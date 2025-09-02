import 'dart:ui';
import 'dart:math';

import 'package:flutter/gestures.dart';
import 'package:flutter/scheduler.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/ax_shadow.dart';
import '../../utils/format_time.dart';
import '../../utils/api/play_next.dart';
import '../../utils/api/play_previous.dart';
import '../../utils/router/router_aware_flyout_controller.dart';
import '../../widgets/ax_pressure.dart';
import '../../widgets/tile/cover_art.dart';
import '../../widgets/navigation_bar/utils/activate_link_action.dart';
import '../../screens/cover_wall/widgets/small_screen_playing_track_command_bar_container.dart';
import '../../providers/status.dart';
import '../../providers/responsive_providers.dart';

import './constants/playback_controller_height.dart';

import 'cover_wall_button.dart';

double _lerpDouble(double a, double b, double t) {
  return a * (1.0 - t) + b * t;
}

enum DragDirection { up, right, down, left }

class CoverArtDisk extends StatefulWidget {
  const CoverArtDisk({super.key});

  @override
  CoverArtDiskState createState() => CoverArtDiskState();
}

class CoverArtDiskState extends State<CoverArtDisk>
    with SingleTickerProviderStateMixin {
  final FocusNode _focusNode = FocusNode(debugLabel: 'Cover Art Disk');
  final _contextController = RouterAwareFlyoutController();
  final _contextAttachKey = GlobalKey();

  late final Ticker _ticker;

  // Current actual rotation angle
  double _currentRotation = 0.0;
  // Target rotation angle
  double _targetRotation = 0.0;
  // Last update timestamp
  DateTime? _lastUpdateTime;

  // Animation configuration
  static const double rotationsPerSecond = 0.04; // Rotations per second
  static const double radiansPerSecond =
      rotationsPerSecond * 2 * pi; // Radians per second
  static const double lerpFactor =
      0.04; // Angle interpolation factor, controls smoothness

  bool _isHovered = false;
  bool _isFocused = false;

  String? _currentPath;
  bool _isSwitching = false;

  static const Duration switchDuration = Duration(milliseconds: 300);

  void _handleFocusHighlight(bool value) {
    setState(() {
      _isFocused = value;
    });
  }

  void _handleHoverHighlight(bool value) {
    setState(() {
      _isHovered = value;
    });
  }

  @override
  void initState() {
    super.initState();

    _ticker = createTicker(_onTick);
    _ticker.start();
  }

  // Interpolate between two angles
  double _lerpAngle(double current, double target, double t) {
    // Calculate the shortest path
    double diff = (target - current) % (2 * pi);
    if (diff > pi) {
      diff -= 2 * pi;
    } else if (diff < -pi) {
      diff += 2 * pi;
    }

    return current + diff * t;
  }

  void _onTick(Duration _) {
    final now = DateTime.now();
    if (_lastUpdateTime != null) {
      final status = Provider.of<PlaybackStatusProvider>(
        context,
        listen: false,
      ).playbackStatus;
      final elapsed =
          now.difference(_lastUpdateTime!).inMicroseconds /
          Duration.microsecondsPerSecond;

      if (status.state == 'Playing') {
        // Update target angle
        _targetRotation += radiansPerSecond * elapsed;
        _targetRotation %= (2 * pi);
      }

      setState(() {
        // Use lerp to smoothly transition to the target angle and offset
        _currentRotation = _lerpAngle(
          _currentRotation,
          _targetRotation,
          lerpFactor,
        );

        _finalOffsetX = _lerpDouble(
          _finalOffsetX,
          0 - _baseOffsetX + _targetDragOffsetX,
          dragLerpFactor,
        );
        _finalOffsetY = _lerpDouble(
          _finalOffsetY,
          0 - _baseOffsetY + _targetDragOffsetY,
          dragLerpFactor,
        );
      });
    }
    _lastUpdateTime = now;
  }

  Future<void> _handlePathChange(
    String? newPath,
    double size,
    bool isCar,
    Duration duration,
  ) async {
    if (_currentPath == newPath || _isSwitching) return;

    if (!mounted) return;

    setState(() {
      _isSwitching = true;
    });

    await Future.delayed(duration);

    if (!mounted) return;

    setState(() {
      _currentPath = newPath;
    });

    await Future.delayed(duration);

    if (!mounted) return;

    setState(() {
      _isSwitching = false;
    });
  }

  @override
  void dispose() {
    _ticker.dispose();
    _focusNode.dispose();
    _contextController.dispose();
    super.dispose();
  }

  double _baseOffsetX = 0.0;
  double _baseOffsetY = 0.0;

  bool _isDragging = false;
  double _targetDragOffsetX = 0.0;
  double _targetDragOffsetY = 0.0;

  Offset? _startPosition;
  static const double _dragThreshold = 10.0;
  static const double dragLerpFactor = 0.15;

  double _finalOffsetX = 0.0;
  double _finalOffsetY = 0.0;

  _onPressed() {
    showCoverArtWall();
  }

  void _onSwitch(DragDirection direction) {
    if (direction == DragDirection.right || direction == DragDirection.up) {
      playPrevious();
    } else {
      playNext();
    }
  }

  void _handlePointerDown(PointerDownEvent event) {
    _pointerDownButton = event.buttons;
    _startPosition = event.position;
    _targetDragOffsetX = 0;
    _targetDragOffsetY = 0;
    _isDragging = false;
  }

  void _handlePointerMove(PointerMoveEvent event) {
    if (_startPosition == null) return;

    final delta = _startPosition! - event.position;

    // If dragging hasn't started yet, check if it exceeds the threshold
    if (!_isDragging && delta.distance > _dragThreshold) {
      _isDragging = true;
    }

    if (_isDragging) {
      setState(() {
        final isCar = Provider.of<ResponsiveProvider>(
          context,
          listen: false,
        ).smallerOrEqualTo(DeviceType.car, false);

        if (isCar) {
          _targetDragOffsetX = 0;
          _targetDragOffsetY = delta.dy;
        } else {
          _targetDragOffsetX = delta.dx;
          _targetDragOffsetY = 0;
        }
      });
    }
  }

  void _handlePointerUp(PointerUpEvent event) {
    if (_isDragging) {
      // Check if the switch is triggered
      final size = min(
        MediaQuery.of(context).size.height,
        MediaQuery.of(context).size.width,
      );

      if ((_finalOffsetX * _finalOffsetX + _finalOffsetY * _finalOffsetY) >
          (size / 4) * (size / 4)) {
        final isCar = Provider.of<ResponsiveProvider>(
          context,
          listen: false,
        ).smallerOrEqualTo(DeviceType.car, false);

        if (isCar) {
          _onSwitch(
            _targetDragOffsetY > 0 ? DragDirection.up : DragDirection.down,
          );
        } else {
          _onSwitch(
            _targetDragOffsetX > 0 ? DragDirection.left : DragDirection.right,
          );
        }
      }

      setState(() {
        _isDragging = false;
        _targetDragOffsetX = 0;
        _targetDragOffsetY = 0;
      });
    } else if (_pointerDownButton == kPrimaryButton) {
      showCoverArtWall();
    } else if (_pointerDownButton == kSecondaryButton) {
      _showContextMenu(event);
    }
    _startPosition = null;
  }

  _showContextMenu(PointerUpEvent event) {
    // This calculates the position of the flyout according to the parent navigator
    final targetContext = _contextAttachKey.currentContext;
    if (targetContext == null) return;
    final box = targetContext.findRenderObject() as RenderBox;
    final position = box.localToGlobal(
      event.localPosition,
      ancestor: Navigator.of(context).context.findRenderObject(),
    );

    _contextController.showFlyout(
      position: position,
      builder: (context) {
        return FlyoutContent(
          child: SmallScreenPlayingTrackCommandBarContainer(shadows: const []),
        );
      },
    );
  }

  int _pointerDownButton = 0;

  @override
  Widget build(BuildContext context) {
    final statusProvider = Provider.of<PlaybackStatusProvider>(context);
    final status = statusProvider.playbackStatus;
    final notReady = statusProvider.notReady;
    final r = Provider.of<ResponsiveProvider>(context);
    final screen = Provider.of<ScreenSizeProvider>(context).screenSize;

    final theme = FluentTheme.of(context);
    final size = min(screen.height, screen.width);

    final isCar = r.smallerOrEqualTo(DeviceType.car, false);
    final isWatch = smallerThanWatch(screen);

    if (_currentPath != status.coverArtPath) {
      _handlePathChange(
        status.coverArtPath,
        size,
        isCar,
        theme.fastAnimationDuration,
      );
    }

    if (isWatch) return Container();

    _baseOffsetY = isCar
        ? 0.0
        : notReady || _isSwitching
        ? size * 1.2
        : max(size / 5 * 3, size - playbackControllerHeight + 8);

    _baseOffsetX = !isCar
        ? 0.0
        : notReady || _isSwitching
        ? screen.height / 2 + size * 1.2
        : (screen.height / 2) + screen.height / 6;

    const radius = 512;

    Color borderColor;
    List<BoxShadow>? boxShadow;

    if (_isFocused) {
      borderColor = theme.accentColor;
      boxShadow = [
        BoxShadow(
          color: theme.accentColor.withValues(alpha: 0.5),
          blurRadius: 10,
          spreadRadius: 2,
        ),
      ];
    } else if (_isHovered) {
      borderColor = theme.resources.controlStrokeColorDefault;
    } else {
      borderColor = theme.resources.controlStrokeColorSecondary;
    }

    return Positioned(
      right: _finalOffsetX,
      bottom: _finalOffsetY,
      child: Transform(
        transform: Matrix4.identity()
          ..scaleByDouble(0.9, 0.9, 0.9, 1.0)
          ..rotateZ(_currentRotation),
        alignment: Alignment.center,
        child: Listener(
          onPointerDown: _handlePointerDown,
          onPointerMove: _handlePointerMove,
          onPointerUp: _handlePointerUp,
          child: FlyoutTarget(
            key: _contextAttachKey,
            controller: _contextController.controller,
            child: FocusableActionDetector(
              focusNode: _focusNode,
              onShowFocusHighlight: _handleFocusHighlight,
              onShowHoverHighlight: _handleHoverHighlight,
              actions: {
                ActivateIntent: ActivateLinkAction(context, _onPressed),
              },
              child: Container(
                decoration: BoxDecoration(
                  borderRadius: BorderRadius.circular(512),
                  boxShadow: axShadow(10),
                ),
                child: SizedBox(
                  width: size,
                  height: size,
                  child: AspectRatio(
                    aspectRatio: 1,
                    child: AxPressure(
                      child: ClipRRect(
                        borderRadius: BorderRadius.circular(512),
                        child: BackdropFilter(
                          filter: ImageFilter.blur(sigmaX: 5.0, sigmaY: 5.0),
                          child: AnimatedContainer(
                            duration: theme.fastAnimationDuration,
                            width: double.infinity,
                            height: double.infinity,
                            decoration: BoxDecoration(
                              border: Border.all(color: borderColor, width: 5),
                              borderRadius: BorderRadius.circular(512),
                              boxShadow: _isFocused ? boxShadow : null,
                            ),
                            child: ClipRRect(
                              borderRadius: BorderRadius.circular(radius - 1),
                              child: CoverArt(
                                size: size,
                                path: _currentPath,
                                hint: (
                                  status.album ?? "",
                                  status.artist ?? "",
                                  'Total Time ${formatTime(status.duration)}',
                                ),
                              ),
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
