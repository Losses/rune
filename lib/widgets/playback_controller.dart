import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../messages/playback.pb.dart';

String formatTime(double seconds) {
  int totalSeconds = seconds.floor();

  int minutes = totalSeconds ~/ 60;
  int remainingSeconds = totalSeconds % 60;

  String minutesStr = minutes.toString().padLeft(2, '0');
  String secondsStr = remainingSeconds.toString().padLeft(2, '0');

  return '$minutesStr:$secondsStr';
}

class PlaybackController extends StatefulWidget {
  const PlaybackController({super.key});

  @override
  PlaybackControllerState createState() => PlaybackControllerState();
}

class PlaybackControllerState extends State<PlaybackController> {
  @override
  Widget build(BuildContext context) {
    return StreamBuilder(
      stream: PlaybackStatus.rustSignalStream, // GENERATED
      builder: (context, snapshot) {
        if (!snapshot.hasData) {
          return const Text("No playback data received yet");
        }

        final playbackStatus = snapshot.data!.message;
        final state = playbackStatus.state;
        final progressSeconds = playbackStatus.progressSeconds;
        final progressPercentage = playbackStatus.progressPercentage;
        final title = playbackStatus.title;
        final duration = playbackStatus.duration;

        return Row(
          children: [
            Expanded(
              child: Center(
                child: Container(
                  constraints:
                      const BoxConstraints(minWidth: 200, maxWidth: 400),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(title),
                      Slider(
                        value: progressPercentage * 100,
                        onChanged: (v) =>
                            SeekRequest(positionSeconds: (v / 100) * duration)
                                .sendSignalToRust(),
                        style: const SliderThemeData(useThumbBall: false),
                      ),
                      Row(
                        mainAxisAlignment: MainAxisAlignment.spaceBetween,
                        children: [
                          Text(formatTime(progressSeconds)),
                          Text('-${formatTime(duration - progressSeconds)}'),
                        ],
                      )
                    ],
                  ),
                ),
              ),
            ),
            Row(
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                IconButton(
                  onPressed: () {
                    PreviousRequest().sendSignalToRust(); // GENERATED
                  },
                  icon: const Icon(Symbols.skip_previous),
                ),
                IconButton(
                  onPressed: () {
                    switch (state) {
                      case "Paused" || "Stopped":
                        PlayRequest().sendSignalToRust(); // GENERATED
                      case "Playing":
                        PauseRequest().sendSignalToRust(); // GENERATED
                    }
                  },
                  icon: state == "Playing"
                      ? const Icon(Symbols.pause)
                      : const Icon(Symbols.play_arrow),
                ),
                IconButton(
                  onPressed: () {
                    NextRequest().sendSignalToRust(); // GENERATED
                  },
                  icon: const Icon(Symbols.skip_next),
                ),
                // IconButton(
                //   onPressed: () {
                //     RemoveRequest(index: 1)
                //         .sendSignalToRust(); // Remove item at index 1
                //   },
                //   icon: const Icon(Symbols.play_arrow),
                // ),
              ],
            ),
          ],
        );
      },
    );
  }
}
