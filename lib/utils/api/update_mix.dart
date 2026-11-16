import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<Mix> updateMix(
  int mixId,
  String name,
  String group,
  PlaybackMode? mode,
  Iterable<(String, String)> queries,
) async {
  final updateRequest = UpdateMixRequest(
    mixId: mixId,
    name: name,
    group: group,
    mode: mode,
    queries: queries
        .map((x) => MixQuery(operator: x.$1, parameter: x.$2))
        .toList(),
  );

  // Listen for the response from Rust
  final rustSignal = await awaitResponse(
    'UpdateMixRequest',
    UpdateMixResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        updateRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response.mix;
}
