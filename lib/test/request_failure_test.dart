import 'dart:async';

import 'package:flutter_test/flutter_test.dart';

import '../bindings/bindings.dart';
import '../l10n/app_localizations_en.dart';
import '../utils/api/request_failure.dart';

void main() {
  const failure = RequestFailedResponse(
    request: 'Wanted',
    kind: RequestErrorKind.notFound,
    detail: 'gone',
  );

  test('response wins and both subscriptions are cancelled', () async {
    final responses = StreamController<int>();
    final failures = StreamController<RequestFailedResponse>();
    final result = awaitResponse(
      'Wanted',
      responses.stream,
      failures: failures.stream,
    );
    responses.add(42);
    expect(await result, 42);
    expect(responses.hasListener, false);
    expect(failures.hasListener, false);
    await responses.close();
    await failures.close();
  });

  test('matching failure preserves kind and detail', () async {
    final responses = StreamController<int>();
    final failures = StreamController<RequestFailedResponse>();
    final result = awaitResponse(
      'Wanted',
      responses.stream,
      failures: failures.stream,
    );
    failures.add(failure);
    await expectLater(
      result,
      throwsA(
        isA<RequestFailure>()
            .having((e) => e.kind, 'kind', RequestErrorKind.notFound)
            .having((e) => e.detail, 'detail', 'gone'),
      ),
    );
    expect(responses.hasListener, false);
    expect(failures.hasListener, false);
    await responses.close();
    await failures.close();
  });

  test('unrelated failure is ignored', () async {
    final responses = StreamController<int>();
    final failures = StreamController<RequestFailedResponse>();
    final result = awaitResponse(
      'Other',
      responses.stream,
      failures: failures.stream,
    );
    failures.add(failure);
    await Future<void>.delayed(Duration.zero);
    responses.add(7);
    expect(await result, 7);
    await responses.close();
    await failures.close();
  });

  test('timeout cancels subscriptions', () async {
    final responses = StreamController<int>();
    final failures = StreamController<RequestFailedResponse>();
    await expectLater(
      awaitResponse(
        'Wanted',
        responses.stream,
        failures: failures.stream,
        timeout: const Duration(milliseconds: 10),
      ),
      throwsA(isA<TimeoutException>()),
    );
    expect(responses.hasListener, false);
    expect(failures.hasListener, false);
    await responses.close();
    await failures.close();
  });

  test('failure messages cover all kinds', () {
    final s = AppLocalizationsEn();
    String message(RequestErrorKind kind) =>
        requestFailureMessage(s, RequestFailure('Wanted', kind, 'detail'));
    expect(message(RequestErrorKind.notFound), 'This item no longer exists');
    expect(message(RequestErrorKind.invalidInput), 'The request was not valid');
    expect(
      message(RequestErrorKind.unavailable),
      "This isn't available right now",
    );
    expect(message(RequestErrorKind.permissionDenied), "This device isn't allowed to do that");
    expect(message(RequestErrorKind.conflict), 'The queue changed; try again');
    expect(message(RequestErrorKind.internal), s.unknownError);
  });

  test('storage busy cause has its localized retry message', () {
    final s = AppLocalizationsEn();
    const failure = RequestFailure(
      'Wanted',
      RequestErrorKind.unavailable,
      'locked',
      RequestFailureCauseStorageBusy(),
    );
    expect(requestFailureMessage(s, failure), 'Storage is busy. Try again.');
  });

  test('certificate mismatch has its localized message', () {
    final s = AppLocalizationsEn();
    const failure = RequestFailure(
      'AddTrustedServerRequest',
      RequestErrorKind.invalidInput,
      'mismatch',
      RequestFailureCauseCertificateMismatch(),
    );
    expect(
      requestFailureMessage(s, failure),
      "This device's certificate does not match",
    );
  });

  test('error text localizes request failures and preserves other errors', () {
    final s = AppLocalizationsEn();
    expect(
      requestErrorText(
        s,
        const RequestFailure('Wanted', RequestErrorKind.notFound, 'gone'),
      ),
      'This item no longer exists',
    );
    expect(
      requestErrorText(s, StateError('broken')),
      'Error: Bad state: broken',
    );
  });

}
