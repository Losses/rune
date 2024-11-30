import '../../messages/all.dart';

Future<RegisterLicenseResponse> registerLicense(String path) async {
  RegisterLicenseRequest(path: path).sendSignalToRust();

  final rustSignal = await RegisterLicenseResponse.rustSignalStream.first;

  return rustSignal.message;
}
