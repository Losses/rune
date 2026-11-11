import 'package:flutter_test/flutter_test.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations_en.dart';
import '../../utils/api/server_availability_test_request.dart';

void main() {
  test('availability failures use localized messages', () {
    final s = AppLocalizationsEn();

    expect(
      serverAvailabilityFailureMessage(
        s,
        ServerAvailabilityTestFailure.addressMalformed,
      ),
      s.requestInvalid,
    );
    expect(
      serverAvailabilityFailureMessage(
        s,
        ServerAvailabilityTestFailure.noHostReachable,
      ),
      s.requestUnavailable,
    );
    expect(
      serverAvailabilityFailureMessage(
        s,
        ServerAvailabilityTestFailure.tlsFingerprintMismatch,
      ),
      s.requestCertificateMismatch,
    );
    expect(
      serverAvailabilityFailureMessage(
        s,
        ServerAvailabilityTestFailure.timedOut,
      ),
      s.requestUnavailable,
    );
  });
}
