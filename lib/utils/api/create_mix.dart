import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<Mix> createMix(
  String name,
  String group,
  PlaybackMode? mode,
  Iterable<(String, String)> queries,
) async {
  final createRequest = CreateMixRequest(
    name: name,
    group: group.isEmpty ? 'Favorite' : group,
    mode: mode,
    queries: queries
        .map((x) => MixQuery(operator: x.$1, parameter: x.$2))
        .toList(),
  );

  // Listen for the response from Rust
  final rustSignal = await awaitResponse(
    'CreateMixRequest',
    CreateMixResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        createRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response.mix;
}
