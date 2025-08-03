import '../../bindings/bindings.dart';

void playPause() async {
  PauseRequest().sendSignalToRust();
}
