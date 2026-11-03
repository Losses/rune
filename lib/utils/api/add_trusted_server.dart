import 'request_failure.dart';
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

  final rustSignal = await awaitResponse(
    'AddTrustedServerRequest',
    AddTrustedServerResponse.rustSignalStream,
    send: () => removeTrustRequest.sendSignalToRust(),
  );
  final response = rustSignal.message;

  if (!response.success) {
    throw response.error;
  }

  return response.success;
}
