import '../../messages/library_manage.pb.dart';

Future<void> scanLibrary(String path) async {
  ScanAudioLibraryRequest(path: path).sendSignalToRust();
}
