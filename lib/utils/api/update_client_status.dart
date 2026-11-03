import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<bool> updateClientStatus(
  String fingerprint, ClientStatus status, {DevicePreset? preset}
) async {
  final rustSignal = await awaitResponse(
    'UpdateClientStatusRequest',
    UpdateClientStatusResponse.rustSignalStream,
    send: () => UpdateClientStatusRequest(
      fingerprint: fingerprint,
      status: status,
      preset: preset,
    ).sendSignalToRust(),
  );
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }

  return response.success;
}
