import 'package:rune/messages/connection.pb.dart';

Future<(bool, String?)> setMediaLibraryPath(String path) async {
  SetMediaLibraryPathRequest(path: path).sendSignalToRust();

  while (true) {
    final rustSignal = await SetMediaLibraryPathResponse.rustSignalStream.first;
    final response = rustSignal.message;

    if (response.path == path) {
      return (response.success, response.error);
    }
  }
}
