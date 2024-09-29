import '../../messages/playback.pb.dart';

void playNext() async {
  NextRequest().sendSignalToRust();
}
