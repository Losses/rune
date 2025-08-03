import '../../bindings/bindings.dart';

Future<bool> updateClientStatus(
  String fingerprint,
  ClientStatus status,
) async {
  UpdateClientStatusRequest(
    fingerprint: fingerprint,
    status: status,
  ).sendSignalToRust();

  final rustSignal = await UpdateClientStatusResponse.rustSignalStream.first;
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }

  return response.success;
}
