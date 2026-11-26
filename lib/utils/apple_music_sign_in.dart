import 'dart:async';

import 'package:flutter/foundation.dart';

import '../bindings/bindings.dart';
import '../l10n/app_localizations.dart';
import 'api/request_failure.dart';
import 'device_sign_in.dart';
import 'hosted_player/apple_music_host.dart';
import 'hosted_player/hosted_player.dart';
import 'library_registry_messages.dart';
import 'platform.dart';

/// How an Apple Music library signs in: through the MusicKit host of the macOS,
/// iOS or Android runner.
const appleMusicSignInMethod = DeviceSignInMethod(
  host: AppleMusicHost.family,
  authorizationMessage: appleMusicAuthorizationMessage,
  signInMessage: appleMusicSignInFailureMessage,
);

/// Whether Rune runs on an iPhone or iPad. An iOS build that runs on a Mac
/// speaks of the Mac.
bool get _onIOS =>
    defaultTargetPlatform == TargetPlatform.iOS && iosDevice != IosDevice.mac;

bool get _onAndroid => defaultTargetPlatform == TargetPlatform.android;

/// What the Apple Music library says about where it plays: this Mac, this
/// iPhone or iPad, or this Android device.
String appleMusicSubtitleText(AppLocalizations s) => _onAndroid
    ? s.appleMusicSubtitleAndroid
    : _onIOS
    ? s.appleMusicSubtitleIOS
    : s.appleMusicSubtitle;

/// What to tell the user when the host gave nothing to sign in with.
String appleMusicAuthorizationMessage(
  AppLocalizations s,
  HostAuthorizationFailure failure,
) => switch (failure) {
  HostAuthorizationFailure.refused =>
    _onAndroid
        ? s.appleMusicAccessDeniedAndroid
        : _onIOS
        ? s.appleMusicAccessDeniedIOS
        : s.appleMusicAccessDenied,
  HostAuthorizationFailure.noToken ||
  HostAuthorizationFailure.unavailable => s.appleMusicNoToken,
  HostAuthorizationFailure.account =>
    _onAndroid
        ? s.appleMusicAccountUnavailableAndroid
        : _onIOS
        ? s.appleMusicAccountUnavailableIOS
        : s.appleMusicAccountUnavailable,
  HostAuthorizationFailure.privacy => s.appleMusicPrivacyNotice,
  // The system an iOS build needs is that of iOS, on a Mac too. Android has no
  // system version to ask for: it lacks the Apple Music app.
  HostAuthorizationFailure.unsupported =>
    _onAndroid
        ? s.appleMusicNeedsAppAndroid
        : defaultTargetPlatform == TargetPlatform.iOS
        ? s.appleMusicNeedsNewerIOS
        : s.appleMusicNeedsNewerMacOS,
};

/// What to tell the user about an Apple Music registration or sign-in the hub
/// refused.
String appleMusicSignInFailureMessage(AppLocalizations s, Object error) =>
    switch (error) {
      RequestFailure(
        cause: RequestFailureCauseLibraryRegistrationFailed(:final kind),
      ) =>
        switch (kind) {
          RegisterLibraryFailureKind.remoteNotAuthorized =>
            s.appleMusicSignInFailed,
          RegisterLibraryFailureKind.invalidSource => s.appleMusicNoToken,
          RegisterLibraryFailureKind.remoteUnreachable =>
            s.appleMusicUnreachable,
          RegisterLibraryFailureKind.remoteProtocol =>
            s.appleMusicUnexpectedResponse,
          RegisterLibraryFailureKind.invalidRecord => s.libraryRecordNotFound,
          _ => libraryRegistryFailureMessage(s, kind),
        },
      RequestFailure() => requestFailureMessage(s, error),
      // The hub did not answer in time, as it does not while Apple cannot be
      // reached.
      TimeoutException() => s.appleMusicUnreachable,
      _ => s.unknownError,
    };
