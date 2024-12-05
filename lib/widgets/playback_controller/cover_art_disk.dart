import 'dart:ui';
import 'dart:math';

import 'package:flutter/gestures.dart';
import 'package:flutter/scheduler.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/ax_shadow.dart';
import '../../utils/format_time.dart';
import '../../widgets/ax_pressure.dart';
import '../../widgets/tile/cover_art.dart';
import '../../widgets/navigation_bar/utils/activate_link_action.dart';
import '../../screens/cover_wall/widgets/small_screen_playing_track_command_bar_container.dart';
import '../../providers/status.dart';
import '../../providers/responsive_providers.dart';

import './constants/playback_controller_height.dart';

import 'cover_wall_button.dart';

class CoverArtDisk extends StatefulWidget {
  const CoverArtDisk({super.key});

  @override
  CoverArtDiskState createState() => CoverArtDiskState();
}

class CoverArtDiskState extends State<CoverArtDisk>
    with SingleTickerProviderStateMixin {
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

  final FocusNode _focusNode = FocusNode(debugLabel: 'Cover Art Disk');
  final _contextController = FlyoutController();
  final _contextAttachKey = GlobalKey();

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
      final status = Provider.of<PlaybackStatusProvider>(context, listen: false)
          .playbackStatus;
      final elapsed = now.difference(_lastUpdateTime!).inMicroseconds /
          Duration.microsecondsPerSecond;

      if (status.state == 'Playing') {
        // Update target angle
        _targetRotation += radiansPerSecond * elapsed;
        _targetRotation %= (2 * pi);
      }

      setState(() {
        // Use lerp to smoothly transition to the target angle
        _currentRotation =
            _lerpAngle(_currentRotation, _targetRotation, lerpFactor);
      });
    }
    _lastUpdateTime = now;
  }

  @override
  void dispose() {
    _ticker.dispose();
    _focusNode.dispose();
    _contextController.dispose();
    super.dispose();
  }

  onPressed() {
    showCoverArtWall();
  }

  showContextMenu(PointerUpEvent event) {
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

    if (isWatch) return Container();

    final duration = notReady
        ? const Duration(milliseconds: 0)
        : theme.fastAnimationDuration;

    final translateY = isCar
        ? 0.0
        : notReady
            ? size * 1.2
            : max(size / 5 * 3, size - playbackControllerHeight + 8);

    final translateX = !isCar
        ? 0.0
        : notReady
            ? screen.height / 2 + size * 1.2
            : (screen.height / 2) + screen.height / 6;

    const radius = 512;

    Color borderColor;
    List<BoxShadow>? boxShadow;

    if (_isFocused) {
      borderColor = theme.accentColor;
      boxShadow = [
        BoxShadow(
          color: theme.accentColor.withOpacity(0.5),
          blurRadius: 10,
          spreadRadius: 2,
        ),
      ];
    } else if (_isHovered) {
      borderColor = theme.resources.controlStrokeColorDefault;
    } else {
      borderColor = theme.resources.controlStrokeColorSecondary;
    }

    return AxPressure(
      child: TweenAnimationBuilder<double>(
        tween: Tween<double>(begin: translateY, end: translateY),
        duration: duration,
        curve: Curves.easeInOut,
        builder: (context, animatedTranslateY, child) {
          return TweenAnimationBuilder<double>(
            tween: Tween<double>(begin: translateX, end: translateX),
            duration: duration,
            curve: Curves.easeInOut,
            builder: (context, animatedTranslateX, child) {
              return Transform(
                transform: Matrix4.identity()
                  ..translate(animatedTranslateX, animatedTranslateY)
                  ..scale(0.9)
                  ..rotateZ(_currentRotation),
                alignment: Alignment.center,
                child: Listener(
                  onPointerDown: (event) {
                    _pointerDownButton = event.buttons;
                  },
                  onPointerUp: (event) {
                    if (_pointerDownButton == kPrimaryButton) {
                      showCoverArtWall();
                    } else if (_pointerDownButton == kSecondaryButton) {
                      showContextMenu(event);
                    }
                  },
                  child: child,
                ),
              );
            },
            child: child,
          );
        },
        child: FlyoutTarget(
          key: _contextAttachKey,
          controller: _contextController,
          child: FocusableActionDetector(
            focusNode: _focusNode,
            onShowFocusHighlight: _handleFocusHighlight,
            onShowHoverHighlight: _handleHoverHighlight,
            actions: {
              ActivateIntent: ActivateLinkAction(context, onPressed),
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
                  child: ClipRRect(
                    borderRadius: BorderRadius.circular(512),
                    child: BackdropFilter(
                      filter: ImageFilter.blur(
                        sigmaX: 5.0,
                        sigmaY: 5.0,
                      ),
                      child: AnimatedContainer(
                        duration: theme.fastAnimationDuration,
                        width: double.infinity,
                        height: double.infinity,
                        decoration: BoxDecoration(
                          border: Border.all(
                            color: borderColor,
                            width: 5,
                          ),
                          borderRadius: BorderRadius.circular(512),
                          boxShadow: _isFocused ? boxShadow : null,
                        ),
                        child: ClipRRect(
                          borderRadius: BorderRadius.circular(radius - 1),
                          child: CoverArt(
                            size: size,
                            path: status.coverArtPath,
                            hint: (
                              status.album,
                              status.artist,
                              'Total Time ${formatTime(status.duration)}'
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
