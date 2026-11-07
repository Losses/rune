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
