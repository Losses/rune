import '../../bindings/bindings.dart';

void sfxPlay(String path) async {
  SfxPlayRequest(path: path).sendSignalToRust();
}
