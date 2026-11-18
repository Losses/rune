import 'dart:async';

import 'package:flutter_test/flutter_test.dart';

import '../bindings/bindings.dart';
import '../l10n/app_localizations_en.dart';
import '../utils/api/request_failure.dart';

void main() {
  final s = AppLocalizationsEn();

  test('a request that got no answer in time reads as unavailable', () {
    expect(
      requestErrorText(s, TimeoutException('no answer')),
      s.requestUnavailable,
    );
  });

  test('a failure the hub reported reads as its own message', () {
    const failure = RequestFailure(
      'QueryMixTracksRequest',
      RequestErrorKind.notFound,
      'gone',
    );
    expect(requestErrorText(s, failure), requestFailureMessage(s, failure));
  });
}
