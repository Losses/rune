import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<List<Mix>> getAllMixes() async {
  final fetchRequest = FetchAllMixesRequest();

  // Listen for the response from Rust
  final rustSignal = await awaitResponse(
    'FetchAllMixesRequest',
    FetchAllMixesResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        fetchRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response.mixes;
}
