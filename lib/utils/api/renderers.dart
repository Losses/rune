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

Future<void> attachRenderer(String fingerprint) async {
  await awaitResponse(
    'AttachRendererRequest',
    RendererAttachmentChanged.rustSignalStream,
    send: () =>
        AttachRendererRequest(fingerprint: fingerprint).sendSignalToRust(),
  );
}

Future<void> detachRenderer() async {
  await awaitResponse(
    'DetachRendererRequest',
    RendererAttachmentChanged.rustSignalStream,
    send: () => const DetachRendererRequest().sendSignalToRust(),
  );
}
