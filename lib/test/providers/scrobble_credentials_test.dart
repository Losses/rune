import 'package:flutter_test/flutter_test.dart';
import 'package:flutter/widgets.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/scrobble.dart';

void main() {
  late AppLocalizations strings;

  AuthenticateSingleServiceResultFailure failureResult(
    ScrobbleAuthenticationFailure failure,
  ) => AuthenticateSingleServiceResultFailure(failure: failure);

  setUpAll(() async {
    strings = await AppLocalizations.delegate.load(const Locale('en'));
  });

  test('saved metadata serializes only a credential reference', () {
    final item = LoginRequestItem(
      serviceId: 'ListenBrainz',
      username: 'listener',
      secretInput: const ScrobbleSecretInputSaved(
        credentialRef: 'd22b1c45-330c-4a0f-9fe1-51210fafe741',
      ),
    );

    final saved = item.toMap();
    expect(saved, {
      'serviceId': 'ListenBrainz',
      'username': 'listener',
      'credential_ref': 'd22b1c45-330c-4a0f-9fe1-51210fafe741',
    });
  });

  test('old secret settings do not parse as credential references', () {
    expect(
      () => itemFromMap({
        'serviceId': 'LastFm',
        'username': 'listener',
        'password': 'old-password',
        'api_key': 'old-api-key',
        'api_secret': 'old-api-secret',
      }),
      throwsA(isA<TypeError>()),
    );
  });

  test('credentials rejected reaches localized Dart login state', () {
    expect(
      scrobbleAuthenticationFailureMessage(
        failureResult(const ScrobbleAuthenticationFailureCredentialsRejected()),
        strings,
      ),
      strings.scrobbleCredentialsRejected,
    );
  });

  test('unreachable service reaches localized Dart login state', () {
    expect(
      scrobbleAuthenticationFailureMessage(
        failureResult(const ScrobbleAuthenticationFailureServiceUnreachable()),
        strings,
      ),
      strings.scrobbleServiceUnreachable,
    );
  });

  test('missing Last.fm API key reaches localized Dart login state', () {
    expect(
      scrobbleAuthenticationFailureMessage(
        failureResult(
          const ScrobbleAuthenticationFailureLastFmApiCredentialsMissing(
            field: LastFmApiCredentialField.apiKey,
          ),
        ),
        strings,
      ),
      strings.scrobbleLastFmApiCredentialsMissing(strings.apiKey),
    );
  });

  test('invalid Last.fm API credentials reach localized Dart login state', () {
    expect(
      scrobbleAuthenticationFailureMessage(
        failureResult(
          const ScrobbleAuthenticationFailureLastFmApiCredentialsInvalid(),
        ),
        strings,
      ),
      strings.scrobbleLastFmApiCredentialsInvalid,
    );
  });

  test('other failure preserves only its redacted detail in Dart state', () {
    expect(
      scrobbleAuthenticationFailureMessage(
        failureResult(
          const ScrobbleAuthenticationFailureOther(
            detail: 'HTTP 503: <redacted>',
          ),
        ),
        strings,
      ),
      '${strings.loginFailedSubtitle}: HTTP 503: <redacted>',
    );
  });
}
