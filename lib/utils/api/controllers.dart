import '../../bindings/bindings.dart';
import 'request_failure.dart';

Future<List<AttachedControllerInfo>> listControllers() async {
  final response = await awaitResponse(
    'ListControllersRequest',
    ListControllersResponse.rustSignalStream,
    send: () => const ListControllersRequest().sendSignalToRust(),
  );
  return response.message.controllers;
}

Future<void> detachController(String id) async {
  await awaitResponse<void>(
    'DetachControllerRequest',
    ControllersUpdated.rustSignalStream
        .map((event) => event.message)
        .where(
          (update) =>
              update.controllers.every((controller) => controller.id != id),
        )
        .map((_) {}),
    send: () => DetachControllerRequest(id: id).sendSignalToRust(),
  );
}
