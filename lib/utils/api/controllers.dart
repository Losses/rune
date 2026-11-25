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

/// Completes once the hub has detached the controller.
Future<void> detachController(
  String id, {
  Stream<DetachControllerResponse>? responses,
  void Function(DetachControllerRequest request)? send,
}) async {
  await awaitResponse(
    'DetachControllerRequest',
    responses ??
        DetachControllerResponse.rustSignalStream.map((event) => event.message),
    responseCallId: (response) => response.callId?.toInt(),
    send: (callId) {
      final request = DetachControllerRequest(id: id, callId: callId);
      if (send != null) {
        send(request);
      } else {
        request.sendSignalToRust();
      }
    },
  );
}
