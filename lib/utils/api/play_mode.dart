import '../../messages/all.dart';

void playMode(int mode) async {
  SetPlaybackModeRequest(mode: mode).sendSignalToRust();
}
