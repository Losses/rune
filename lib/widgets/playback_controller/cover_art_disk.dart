import 'dart:ui';
import 'dart:math';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/utils/ax_shadow.dart';

import '../../utils/format_time.dart';
import '../../widgets/ax_pressure.dart';
import '../../widgets/tile/cover_art.dart';
import '../../widgets/navigation_bar/utils/activate_link_action.dart';
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
  late AnimationController _controller;

  bool _isHovered = false;
  bool _isFocused = false;

  final FocusNode _focusNode = FocusNode(debugLabel: 'Cover Art Disk');

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
    _controller = AnimationController(
      duration: const Duration(seconds: 20),
      vsync: this,
    )..repeat();
  }

  @override
  void dispose() {
    _controller.dispose();
    _focusNode.dispose();
    super.dispose();
  }

  onPressed() {
    showCoverArtWall();
  }

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
      child: AnimatedBuilder(
        animation: _controller,
        builder: (context, child) {
          return TweenAnimationBuilder<double>(
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
                      ..rotateZ(
                        status?.state == 'Playing'
                            ? _controller.value * 2 * pi
                            : 0,
                      ),
                    alignment: Alignment.center,
                    child: TapRegion(
                      onTapInside: (_) {
                        showCoverArtWall();
                      },
                      child: child,
                    ),
                  );
                },
                child: child,
              );
            },
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
                              path: status?.coverArtPath,
                              hint: status != null
                                  ? (
                                      status.album,
                                      status.artist,
                                      'Total Time ${formatTime(status.duration)}'
                                    )
                                  : null,
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
        },
      ),
    );
  }
}
