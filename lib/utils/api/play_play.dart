import '../../messages/playback.pb.dart';

void playPlay() async {
  PlayRequest().sendSignalToRust();
}
