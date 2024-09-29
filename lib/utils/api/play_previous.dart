import '../../messages/playback.pb.dart';

void playPrevious() async {
  PreviousRequest().sendSignalToRust();
}
