import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<(bool, bool, String?)> testLibraryInitialized(String path) async {
  while (true) {
    final rustSignal = await awaitResponse(
      'TestLibraryInitializedRequest',
      TestLibraryInitializedResponse.rustSignalStream.where(
        (event) => event.message.path == path,
      ),
      send: () => TestLibraryInitializedRequest(path: path).sendSignalToRust(),
    );
    final response = rustSignal.message;

    if (response.path == path) {
      return (response.success, !response.notReady, response.error);
    }
  }
}
