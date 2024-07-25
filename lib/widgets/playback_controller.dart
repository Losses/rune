import 'package:fluent_ui/fluent_ui.dart';

import '../messages/playback.pb.dart';

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
        final artist = playbackStatus.artist;
        final album = playbackStatus.album;
        final title = playbackStatus.title;
        final duration = playbackStatus.duration;

        return Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            Text("Status: $state"),
            Text(
                "Progress: ${progressSeconds.toStringAsFixed(2)} seconds (${(progressPercentage * 100).toStringAsFixed(2)}%)"),
            Text("Artist: $artist"),
            Text("Album: $album"),
            Text("Title: $title"),
            Text("Duration: ${duration.toStringAsFixed(2)} seconds"),
          ],
        );
      },
    );
  }
}
