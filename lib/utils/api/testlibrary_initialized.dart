import '../../bindings/bindings.dart';

Future<(bool, bool, String?)> testLibraryInitialized(String path) async {
  TestLibraryInitializedRequest(path: path).sendSignalToRust();

  while (true) {
    final rustSignal =
        await TestLibraryInitializedResponse.rustSignalStream.first;
    final response = rustSignal.message;

    if (response.path == path) {
      return (response.success, !response.notReady, response.error);
    }
  }
}
