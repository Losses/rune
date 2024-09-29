import '../../messages/playback.pb.dart';

void playMode(int mode) async {
  SetPlaybackModeRequest(mode: mode).sendSignalToRust();
}
