import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../providers/router_path.dart';
import '../../providers/responsive_providers.dart';

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
  Widget build(BuildContext context) {
    final path = Provider.of<RouterPathProvider>(context).path;
    final isCoverArtWall = path == '/cover_wall';

    final r = Provider.of<ResponsiveProvider>(context);

    final largeLayout = isCoverArtWall && r.smallerOrEqualTo(DeviceType.phone);

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
  }
}
