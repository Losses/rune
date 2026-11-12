import '../../bindings/bindings.dart';
import 'request_failure.dart';

Future<List<RendererCandidate>> listRenderers(List<String> libraryIds) async {
  final response = await awaitResponse(
    'ListRenderersRequest',
    ListRenderersResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => ListRenderersRequest(
      libraryIds: libraryIds,
      callId: callId,
    ).sendSignalToRust(),
  );
  return response.message.renderers;
}

/// Moves the queue and position to the renderer [target], or back to this
/// device when [target] is null.
Future<void> handoffRenderer(String? target) async {
  await awaitResponse(
    'HandoffRequest',
    HandoffResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        HandoffRequest(target: target, callId: callId).sendSignalToRust(),
  );
}
