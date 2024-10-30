import '../../messages/sfx.pb.dart';

void sfxPlay(String path) async {
  SfxPlayRequest(path: path).sendSignalToRust();
}
