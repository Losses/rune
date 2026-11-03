import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<RegisterLicenseResponse> registerLicense(String path) async {
  final rustSignal = await awaitResponse(
    'RegisterLicenseRequest',
    RegisterLicenseResponse.rustSignalStream,
    send: () => RegisterLicenseRequest(path: path).sendSignalToRust(),
  );

  return rustSignal.message;
}
