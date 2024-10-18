import 'dart:math';
import 'package:fluent_ui/fluent_ui.dart';

import 'package:provider/provider.dart';

import '../../utils/format_time.dart';
import '../../widgets/ax_pressure.dart';
import '../../widgets/playback_controller/cover_wall_button.dart';
import '../../widgets/tile/tile.dart';
import '../../widgets/tile/cover_art.dart';
import '../../providers/status.dart';
import '../../providers/responsive_providers.dart';

import './constants/playback_controller_height.dart';

class CoverArtDisk extends StatefulWidget {
  const CoverArtDisk({super.key});

  @override
  CoverArtDiskState createState() => CoverArtDiskState();
}

class CoverArtDiskState extends State<CoverArtDisk>
    with SingleTickerProviderStateMixin {
  late AnimationController _controller;

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
    super.dispose();
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
            ? screen.width / 2 + size * 1.2
            : (screen.width / 2) + screen.height / 6;

    return AnimatedBuilder(
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
                  child: child,
                );
              },
              child: child,
            );
          },
          child: SizedBox(
            width: size,
            height: size,
            child: AxPressure(
              child: AspectRatio(
                aspectRatio: 1,
                child: Tile(
                  radius: 512,
                  borderWidth: 5,
                  onPressed: () {
                    showCoverArtWall(context);
                  },
                  child: CoverArt(
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
        );
      },
    );
  }
}
