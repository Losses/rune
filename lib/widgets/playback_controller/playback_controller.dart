import 'dart:io';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/is_cover_art_wall_layout.dart';
import '../../providers/router_path.dart';
import '../../providers/responsive_providers.dart';
import '../../providers/linux_custom_window_controls.dart';
import '../../providers/visualizer_lab.dart';
import '../title_bar/drag_move_window_area.dart';
import '../visualizer/lab/lab_mode_builder.dart';

import 'constants/playback_controller_height.dart';

import 'now_playing.dart';
import 'fft_visualize.dart';
import 'controller_buttons.dart';

class PlaybackController extends StatefulWidget {
  const PlaybackController({super.key});

  @override
  PlaybackControllerState createState() => PlaybackControllerState();
}

const scaleY = 0.9;

class PlaybackControllerState extends State<PlaybackController> {
  @override
  void initState() {
    super.initState();
  }

  @override
  Widget build(BuildContext context) {
    final path = Provider.of<RouterPathProvider>(context).path;
    final isCoverArtWall = isCoverArtWallLayout(path);

    final r = Provider.of<ResponsiveProvider>(context);

    final largeLayout = isCoverArtWall && r.smallerOrEqualTo(DeviceType.phone);

    // In the lab mode the controller is at the window's top, where the title
    // row was: its empty background moves the window, if the window has no
    // native title bar to do that.
    final dragsWindow = watchVisualizerLabMode(context) &&
        (Platform.isMacOS ||
            Platform.isWindows ||
            (Platform.isLinux &&
                context.watch<LinuxCustomWindowControlsProvider>().enabled));

    return SizedBox(
      height: playbackControllerHeight,
      child: Stack(
        fit: StackFit.expand,
        alignment: Alignment.centerRight,
        children: <Widget>[
          if (dragsWindow) const Positioned.fill(child: DragMoveWindowArea()),
          SizedBox.expand(
            child: Center(
              child: Container(
                constraints: const BoxConstraints(
                  minWidth: 1200,
                  maxWidth: 1600,
                ),
                // The spectrum rises from the window's bottom edge, and in
                // the lab mode, with the controller at the top, it hangs from
                // the top edge: only half of it shows either way.
                child: LabModeBuilder(
                  builder: (context, lab, on, child) => Transform(
                    transform: Matrix4.identity()
                      ..scaleByDouble(1.0, scaleY, 1.0, 1.0)
                      ..translateByDouble(
                        0.0,
                        (1 - scaleY) * 100 * (1 - lab),
                        0.0,
                        1.0,
                      ),
                    // The spectrum redraws every frame while music plays;
                    // without a boundary of its own, each redraw repaints the
                    // whole page behind it (the cover wall, its shader, the
                    // scene).
                    child: IgnorePointer(
                      child: RepaintBoundary(
                        child: FFTVisualize(
                          alignment: Alignment.lerp(
                            Alignment.topCenter,
                            Alignment.bottomCenter,
                            lab,
                          )!,
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ),
          if (!largeLayout) const NowPlaying(),
          const ControllerButtons(),
        ],
      ),
    );
  }
}
