import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<bool> updateClientStatus(
  String fingerprint,
  ClientStatus status, {
  DevicePreset? preset,
}) async {
  final rustSignal = await awaitResponse(
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
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }

  return response.success;
}
