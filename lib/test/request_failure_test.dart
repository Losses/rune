import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:rinf/rinf.dart';

import '../bindings/bindings.dart';
import '../l10n/app_localizations_en.dart';
import '../utils/api/request_failure.dart';
import '../utils/api/search_for.dart';
import '../utils/api/remove_item_from_playlist.dart';

void main() {
  RequestFailedResponse failure(int callId) => RequestFailedResponse(
    request: 'Wanted',
    callId: Uint64.fromBigInt(BigInt.from(callId)),
    kind: RequestErrorKind.notFound,
    detail: 'gone',
  );

  test('response wins and both subscriptions are cancelled', () async {
    final responses = StreamController<int>();
    final failures = StreamController<RequestFailedResponse>();
    late Uint64 sentCallId;
    final result = awaitResponse(
      'Wanted',
      responses.stream,
      failures: failures.stream,
      responseCallId: (value) => value,
      send: (callId) => sentCallId = callId,
    );
    responses.add(42);
    responses.add(sentCallId.toInt());
    expect(await result, sentCallId.toInt());
    expect(responses.hasListener, false);
    expect(failures.hasListener, false);
    await responses.close();
    await failures.close();
  });

  test('matching failure preserves kind and detail', () async {
    final responses = StreamController<int>();
    final failures = StreamController<RequestFailedResponse>();
    late Uint64 sentCallId;
    final result = awaitResponse(
      'Wanted',
      responses.stream,
      failures: failures.stream,
      responseCallId: (_) => null,
      send: (callId) => sentCallId = callId,
    );
    failures.add(failure(sentCallId.toInt()));
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

  test(
    'playlist request failure shows the existing localized message',
    () async {
      final responses = StreamController<int>();
      final failures = StreamController<RequestFailedResponse>();
      late Uint64 sentCallId;
      final result = awaitResponse(
        'RemoveItemFromPlaylistRequest',
        responses.stream,
        failures: failures.stream,
        responseCallId: (_) => null,
        send: (callId) => sentCallId = callId,
      );
      failures.add(
        RequestFailedResponse(
          request: 'diagnostic name is ignored',
          callId: sentCallId,
          kind: RequestErrorKind.notFound,
          detail: 'playlist',
        ),
      );
      final failure = await result.then<RequestFailure>(
        (_) => fail('expected a playlist request failure'),
        onError: (Object error) => error as RequestFailure,
      );
      expect(
        requestFailureMessage(AppLocalizationsEn(), failure),
        AppLocalizationsEn().requestItemMissing,
      );
      await responses.close();
      await failures.close();
    },
  );

  test('unrelated failure is ignored', () async {
    final responses = StreamController<int>();
    final failures = StreamController<RequestFailedResponse>();
    late Uint64 sentCallId;
    final result = awaitResponse(
      'Other',
      responses.stream,
      failures: failures.stream,
      responseCallId: (_) => sentCallId.toInt(),
      send: (callId) => sentCallId = callId,
    );
    failures.add(
      failure(sentCallId.toInt() + 1).copyWith(
        callId: () => Uint64.fromBigInt(BigInt.from(sentCallId.toInt() + 1)),
      ),
    );
    responses.add(sentCallId.toInt());
    expect(await result, sentCallId.toInt());
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
        responseCallId: (_) => null,
        send: (_) {},
      ),
      throwsA(isA<TimeoutException>()),
    );
    expect(responses.hasListener, false);
    expect(failures.hasListener, false);
    await responses.close();
    await failures.close();
  });

  test(
    'query and change API helpers surface failures by sent call id',
    () async {
      final queryResponses =
          StreamController<RustSignalPack<SearchForResponse>>();
      final queryFailures = StreamController<RequestFailedResponse>.broadcast(
        sync: true,
      );
      final queryTimer = Stopwatch()..start();
      final query = searchForWithStreams(
        'song',
        fields: const ['tracks'],
        responses: queryResponses.stream,
        failures: queryFailures.stream,
        timeout: const Duration(seconds: 2),
        sendRequest: (request) => queryFailures.add(
          RequestFailedResponse(
            request: 'diagnostic only',
            callId: request.callId!,
            kind: RequestErrorKind.unavailable,
            detail: 'query refused',
          ),
        ),
      );
      await expectLater(
        query,
        throwsA(
          isA<RequestFailure>()
              .having(
                (error) => error.kind,
                'kind',
                RequestErrorKind.unavailable,
              )
              .having((error) => error.detail, 'detail', 'query refused'),
        ),
      );
      expect(queryTimer.elapsed, lessThan(const Duration(seconds: 1)));
      await queryResponses.close();
      await queryFailures.close();

      final changeResponses =
          StreamController<RustSignalPack<RemoveItemFromPlaylistResponse>>();
      final changeFailures = StreamController<RequestFailedResponse>.broadcast(
        sync: true,
      );
      final changeTimer = Stopwatch()..start();
      final change = removeItemFromPlaylistWithStreams(
        7,
        'occurrence-1',
        responses: changeResponses.stream,
        failures: changeFailures.stream,
        sendRequest: (request) => changeFailures.add(
          RequestFailedResponse(
            request: 'diagnostic only',
            callId: request.callId!,
            kind: RequestErrorKind.conflict,
            detail: 'change refused',
          ),
        ),
      );
      await expectLater(
        change,
        throwsA(
          isA<RequestFailure>()
              .having((error) => error.kind, 'kind', RequestErrorKind.conflict)
              .having((error) => error.detail, 'detail', 'change refused'),
        ),
      );
      expect(changeTimer.elapsed, lessThan(const Duration(seconds: 1)));
      await changeResponses.close();
      await changeFailures.close();
    },
  );

  test('failure messages cover all kinds', () {
    final s = AppLocalizationsEn();
    String message(RequestErrorKind kind) =>
        requestFailureMessage(s, RequestFailure('Wanted', kind, 'detail'));
    expect(message(RequestErrorKind.notFound), 'This item no longer exists.');
    expect(message(RequestErrorKind.invalidInput), 'The request is invalid.');
    expect(
      message(RequestErrorKind.unavailable),
      "This is not available right now.",
    );
    expect(
      message(RequestErrorKind.permissionDenied),
      "This device is not allowed to do that.",
    );
    expect(message(RequestErrorKind.conflict), 'The queue changed. Try again.');
    expect(
      message(RequestErrorKind.peerNeedsUpdate),
      'Update Rune on both devices to connect.',
    );
    expect(message(RequestErrorKind.internal), s.unknownError);
  });

  test('running library task failure names the task in localized text', () {
    final s = AppLocalizationsEn();
    expect(
      requestFailureMessage(
        s,
        const RequestFailure(
          'AnalyzeAudioLibraryRequest',
          RequestErrorKind.conflict,
          'Cannot start a library task while scan is running',
          RequestFailureCauseLibraryTaskAlreadyRunning(
            task: LibraryTaskKind.scan,
          ),
        ),
      ),
      'Scanning: This is not available right now.',
    );
  });

  test('peer update failures name the device that must update', () {
    final s = AppLocalizationsEn();
    const kind = RequestErrorKind.peerNeedsUpdate;
    expect(
      requestFailureMessage(
        s,
        const RequestFailure(
          'Connect',
          kind,
          'incompatible',
          RequestFailureCausePeerNeedsUpdate(
            updateSide: ProtocolUpdateSide.peer,
            peerAlias: 'Bedroom',
            localVersion: '1',
            peerVersion: '0',
          ),
        ),
      ),
      'Bedroom needs a Rune update to connect.',
    );
    expect(
      requestFailureMessage(
        s,
        const RequestFailure(
          'Connect',
          kind,
          'incompatible',
          RequestFailureCausePeerNeedsUpdate(
            updateSide: ProtocolUpdateSide.peer,
            localVersion: '1',
            peerVersion: '0',
          ),
        ),
      ),
      'the other device needs a Rune update to connect.',
    );
    expect(
      requestFailureMessage(
        s,
        const RequestFailure(
          'Connect',
          kind,
          'incompatible',
          RequestFailureCausePeerNeedsUpdate(
            updateSide: ProtocolUpdateSide.thisDevice,
            peerAlias: 'Studio',
            localVersion: '0',
            peerVersion: '1',
          ),
        ),
      ),
      'Update Rune on this device to connect to Studio.',
    );
    expect(
      requestFailureMessage(
        s,
        const RequestFailure(
          'Connect',
          kind,
          'incompatible',
          RequestFailureCausePeerNeedsUpdate(
            updateSide: ProtocolUpdateSide.thisDevice,
            localVersion: '0',
            peerVersion: '1',
          ),
        ),
      ),
      'Update Rune on this device to connect to the other device.',
    );
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
      "This device's certificate does not match.",
    );
  });

  test('pairing registration failures use localized typed messages', () {
    final s = AppLocalizationsEn();
    expect(
      requestFailureMessage(
        s,
        const RequestFailure(
          'RegisterDeviceOnServerRequest',
          RequestErrorKind.invalidInput,
          'key mismatch',
          RequestFailureCausePublicKeyMismatch(),
        ),
      ),
      s.requestPublicKeyMismatch,
    );
    expect(
      requestFailureMessage(
        s,
        const RequestFailure(
          'RegisterDeviceOnServerRequest',
          RequestErrorKind.conflict,
          'fingerprint exists',
          RequestFailureCauseRegistrationConflict(),
        ),
      ),
      s.requestRegistrationConflict,
    );
  });

  test('error text localizes request failures and preserves other errors', () {
    final s = AppLocalizationsEn();
    expect(
      requestErrorText(
        s,
        const RequestFailure('Wanted', RequestErrorKind.notFound, 'gone'),
      ),
      'This item no longer exists.',
    );
    expect(
      requestErrorText(s, StateError('broken')),
      'Error: Bad state: broken',
    );
  });
}
