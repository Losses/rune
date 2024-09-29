import '../../messages/playback.pb.dart';

void playPause() async {
  PauseRequest().sendSignalToRust();
}
