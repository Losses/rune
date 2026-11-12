import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<RegisterLicenseResponse> registerLicense(String path) async {
  final rustSignal = await awaitResponse(
    'RegisterLicenseRequest',
    RegisterLicenseResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        RegisterLicenseRequest(path: path, callId: callId).sendSignalToRust(),
  );

  return rustSignal.message;
}
