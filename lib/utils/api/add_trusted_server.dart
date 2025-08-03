import '../../bindings/bindings.dart';

Future<bool> addTrustedServer(
  String fingerprint,
  List<String> hosts,
) async {
  final removeTrustRequest = AddTrustedServerRequest(
    certificate: TrustedServerCertificate(
      fingerprint: fingerprint,
      hosts: hosts,
    ),
  );
  removeTrustRequest.sendSignalToRust();

  final rustSignal = await AddTrustedServerResponse.rustSignalStream.first;
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }

  return response.success;
}
