import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<void> updateClientStatus(
  String fingerprint,
  ClientStatus status, {
  DevicePreset? preset,
}) async {
  await awaitResponse(
    'UpdateClientStatusRequest',
    UpdateClientStatusResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => UpdateClientStatusRequest(
      fingerprint: fingerprint,
      status: status,
      preset: preset,
      callId: callId,
    ).sendSignalToRust(),
  );
}
