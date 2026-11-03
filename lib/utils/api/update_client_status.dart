import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<bool> updateClientStatus(String fingerprint, ClientStatus status) async {
  final rustSignal = await awaitResponse(
    'UpdateClientStatusRequest',
    UpdateClientStatusResponse.rustSignalStream,
    send: () => UpdateClientStatusRequest(
      fingerprint: fingerprint,
      status: status,
    ).sendSignalToRust(),
  );
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }

  return response.success;
}
