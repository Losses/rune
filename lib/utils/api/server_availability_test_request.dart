import 'request_failure.dart';
import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';

String serverAvailabilityFailureMessage(
  AppLocalizations s,
  ServerAvailabilityTestFailure failure,
) =>
    switch (failure) {
      ServerAvailabilityTestFailure.addressMalformed => s.requestInvalid,
      ServerAvailabilityTestFailure.noHostReachable => s.requestUnavailable,
      ServerAvailabilityTestFailure.tlsFingerprintMismatch =>
        s.requestCertificateMismatch,
      ServerAvailabilityTestFailure.timedOut => s.requestUnavailable,
    };

Future<bool> serverAvailabilityTest(List<String> hosts, AppLocalizations s) async {
  final removeTrustRequest = ServerAvailabilityTestRequest(hosts: hosts);

  final rustSignal = await awaitResponse(
    'ServerAvailabilityTestRequest',
    ServerAvailabilityTestResponse.rustSignalStream,
    send: () => removeTrustRequest.sendSignalToRust(),
  );
  final response = rustSignal.message;

  switch (response.outcome) {
    case ServerAvailabilityTestOutcomeSuccess():
      return true;
    case ServerAvailabilityTestOutcomeFailed(:final value):
      throw serverAvailabilityFailureMessage(s, value);
    default:
      throw s.unknownError;
  }
}
