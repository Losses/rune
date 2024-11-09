import '../../messages/all.dart';

void sfxPlay(String path) async {
  SfxPlayRequest(path: path).sendSignalToRust();
}
