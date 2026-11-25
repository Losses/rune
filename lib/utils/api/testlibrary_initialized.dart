import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<bool> testLibraryInitialized(String path) async {
  while (true) {
    final rustSignal = await awaitResponse(
      'TestLibraryInitializedRequest',
      TestLibraryInitializedResponse.rustSignalStream.where(
        (event) => event.message.path == path,
      ),
      responseCallId: (response) => response.message.callId?.toInt(),
      send: (callId) => TestLibraryInitializedRequest(
        path: path,
        callId: callId,
      ).sendSignalToRust(),
    );
    final response = rustSignal.message;

    if (response.path == path) {
      return !response.notReady;
    }
  }
}
