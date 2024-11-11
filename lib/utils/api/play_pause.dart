import '../../messages/all.dart';

void playPause() async {
  PauseRequest().sendSignalToRust();
}
