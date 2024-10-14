import 'dart:math';

import 'package:provider/provider.dart';
import 'package:go_router/go_router.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/format_time.dart';
import '../../widgets/ax_pressure.dart';
import '../../widgets/playback_controller/cover_wall_button.dart';
import '../../widgets/playback_controller/controller_buttons.dart';
import '../../widgets/tile/tile.dart';
import '../../widgets/tile/cover_art.dart';
import '../../widgets/playback_controller/now_playing.dart';
import '../../providers/status.dart';
import '../../providers/responsive_providers.dart';

import './constants/playback_controller_height.dart';
import './fft_visualize.dart';

class PlaybackController extends StatefulWidget {
  const PlaybackController({super.key});

  @override
  PlaybackControllerState createState() => PlaybackControllerState();
}

const scaleY = 0.9;

class PlaybackControllerState extends State<PlaybackController> {
  @override
  Widget build(BuildContext context) {
    final isCoverArtWall = GoRouterState.of(context).fullPath == '/cover_wall';

    final r = Provider.of<ResponsiveProvider>(context);

    final largeLayout = isCoverArtWall && r.smallerOrEqualTo(DeviceType.phone);

    return SmallerOrEqualToScreenSize(
      maxWidth: 340,
      builder: (context, isSmaller) {
        if (isSmaller) return const CoverArtDisk();

        return SizedBox(
          height: playbackControllerHeight,
          child: Stack(
            fit: StackFit.expand,
            alignment: Alignment.centerRight,
            children: <Widget>[
              SizedBox.expand(
                child: Center(
                  child: Container(
                    constraints:
                        const BoxConstraints(minWidth: 1200, maxWidth: 1600),
                    child: Transform(
                      transform: Matrix4.identity()
                        ..scale(1.0, scaleY)
                        ..translate(0.0, (1 - scaleY) * 100),
                      child: const FFTVisualize(),
                    ),
                  ),
                ),
              ),
              if (!largeLayout) const NowPlaying(),
              const ControllerButtons(),
            ],
          ),
        );
      },
    );
  }
}

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

    return LayoutBuilder(
      builder: (context, constraints) {
        final theme = FluentTheme.of(context);
        final size = min(constraints.maxHeight, constraints.maxWidth);

        return AnimatedBuilder(
          animation: _controller,
          builder: (context, child) {
            return TweenAnimationBuilder<double>(
              tween: Tween<double>(
                begin: 0,
                end: notReady
                    ? size * 1.2
                    : max(size / 5 * 3, size - playbackControllerHeight + 8),
              ),
              duration: notReady
                  ? const Duration(milliseconds: 0)
                  : theme.fastAnimationDuration,
              curve: Curves.easeInOut,
              builder: (context, translateY, child) {
                return Transform(
                  transform: Matrix4.identity()
                    ..scale(0.9)
                    ..translate(0.0, translateY)
                    ..rotateZ(
                      status?.state == 'Playing'
                          ? _controller.value * 2 * pi
                          : 0,
                    ),
                  alignment: Alignment.center,
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
      },
    );
  }
}
