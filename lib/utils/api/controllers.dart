import '../../bindings/bindings.dart';
import 'request_failure.dart';

Future<List<AttachedControllerInfo>> listControllers() async {
  final response = await awaitResponse(
    'ListControllersRequest',
    ListControllersResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => ListControllersRequest(callId: callId).sendSignalToRust(),
  );
  return response.message.controllers;
}

Future<void> detachController(String id) async {
  late Uint64 callId;
  await awaitResponse<void>(
    'DetachControllerRequest',
    ControllersUpdated.rustSignalStream
        .map((event) => event.message)
        .where(
          (update) =>
              update.controllers.every((controller) => controller.id != id),
        )
        .map((_) {}),
    responseCallId: (_) => callId.toInt(),
    send: (requestCallId) {
      callId = requestCallId;
      DetachControllerRequest(id: id, callId: requestCallId).sendSignalToRust();
    },
  );
}
