import 'package:fluent_ui/fluent_ui.dart';

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

        return Column(
          children: [
            Column(
              children: [
                Text(title),
                Slider(
                  value: progressPercentage * 100,
                  onChanged: (v) => print(v),
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
            Text("Status: $state"),
          ],
        );
      },
    );
  }
}
