import 'package:flutter_test/flutter_test.dart';

import '../../bindings/bindings.dart';
import '../../utils/api/request_failure.dart';
import '../../utils/theme_color_manager.dart';

void main() {
  test('the color is asked for only when the item\'s library gives it', () {
    expect(asksForColor(const RequestCapabilityAvailable()), isTrue);
    expect(asksForColor(const RequestCapabilityHidden()), isFalse);
    expect(
      asksForColor(
        const RequestCapabilityDisabled(
          reason: CapabilityReason.accountRoleMissing,
        ),
      ),
      isFalse,
    );
  });

  test('a color waiting for a sign-in or missing is no failure', () {
    expect(
      noColorFailure(
        const RequestFailure(
          'GetPrimaryColorByTrackIdRequest',
          RequestErrorKind.permissionDenied,
          'needs a new sign-in',
          RequestFailureCauseServiceSignInRequired(recordId: 'service-record'),
        ),
      ),
      isTrue,
    );
    expect(
      noColorFailure(
        const RequestFailure(
          'GetPrimaryColorByTrackIdRequest',
          RequestErrorKind.notFound,
          'missing',
        ),
      ),
      isTrue,
    );
    // Any other failure still reaches the global handler.
    expect(
      noColorFailure(
        const RequestFailure(
          'GetPrimaryColorByTrackIdRequest',
          RequestErrorKind.permissionDenied,
          'not allowed',
        ),
      ),
      isFalse,
    );
  });
}
