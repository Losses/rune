import '../../bindings/bindings.dart';
import 'request_failure.dart';

Future<List<RendererCandidate>> listRenderers(List<String> libraryIds) async {
  final response = await awaitResponse(
    'ListRenderersRequest',
    ListRenderersResponse.rustSignalStream,
    send: () => ListRenderersRequest(libraryIds: libraryIds).sendSignalToRust(),
  );
  return response.message.renderers;
}

/// Moves the queue and position to the renderer [target], or back to this
/// device when [target] is null.
Future<void> handoffRenderer(String? target) async {
  await awaitResponse(
    'HandoffRequest',
    HandoffResponse.rustSignalStream,
    send: () => HandoffRequest(target: target).sendSignalToRust(),
  );
}
