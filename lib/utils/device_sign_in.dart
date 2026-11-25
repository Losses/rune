import 'dart:async';

import '../bindings/bindings.dart';
import '../l10n/app_localizations.dart';
import '../providers/library_registry.dart';
import 'api/request_failure.dart';
import 'apple_music_sign_in.dart';
import 'hosted_player/hosted_player.dart';
import 'logger.dart';

/// How long a silent authorization may take before it counts as failed, so that a host that
/// never answers holds up no sign-in and no startup.
const silentAuthorizationTimeout = Duration(seconds: 10);

/// How long the hub may take to accept a sign-in before it counts as failed.
const silentSignInTimeout = Duration(seconds: 15);

/// How long a sign-in counts as recent: a failure soon after one is not answered the same way
/// again.
const signInCooldown = Duration(seconds: 60);

/// How many times in a row a sign-in may fail for a reason that may pass by itself, with
/// nothing to show for a network that is down, before it counts as one the user must act on.
const signInFailuresBeforeTelling = 3;

/// How a kind of library signs in on this device: the platform host that authorizes its
/// account, and what to tell the user when it does not sign in.
class DeviceSignInMethod {
  const DeviceSignInMethod({
    required this.host,
    required this.authorizationMessage,
    required this.signInMessage,
  });

  /// The family of the platform host that authorizes the account.
  final String host;

  /// What to tell the user when the host gave nothing to sign in with.
  final String Function(AppLocalizations s, HostAuthorizationFailure failure)
  authorizationMessage;

  /// What to tell the user about a sign-in the hub did not accept.
  final String Function(AppLocalizations s, Object error) signInMessage;

  /// What to tell the user about [failure].
  String message(AppLocalizations s, DeviceSignInFailure failure) =>
      switch (failure.authorization) {
        final authorization? => authorizationMessage(s, authorization),
        null => signInMessage(s, failure.error ?? failure),
      };
}

/// How a library of [kind] signs in on this device, for the kinds that do.
DeviceSignInMethod? deviceSignInMethod(LibraryRecordKindSummary kind) =>
    switch (kind) {
      LibraryRecordKindSummaryAppleMusic() => appleMusicSignInMethod,
      _ => null,
    };

/// The sign-in the hub takes for what the host handed over. It holds the tokens, so it must
/// never be logged or shown.
ServiceSignIn platformSignIn(HostAuthorization authorization) =>
    ServiceSignInPlatform(
      userToken: authorization.token,
      developerToken: authorization.developerToken,
    );

/// Why a library did not sign in on this device: what the host answered instead of an
/// authorization, or what went wrong while the hub signed in with one.
class DeviceSignInFailure implements Exception {
  const DeviceSignInFailure.authorization(
    HostAuthorizationFailure this.authorization,
  ) : error = null,
      repeated = false;

  const DeviceSignInFailure.signIn(Object this.error)
    : authorization = null,
      repeated = false;

  const DeviceSignInFailure._repeated(this.authorization, this.error)
    : repeated = true;

  final HostAuthorizationFailure? authorization;
  final Object? error;

  /// Whether this is a failure that may pass by itself but happened
  /// [signInFailuresBeforeTelling] times in a row, so that it counts as one the user must act
  /// on.
  final bool repeated;

  /// This failure as one that happened so often that the user must be told.
  DeviceSignInFailure asRepeated() =>
      DeviceSignInFailure._repeated(authorization, error);

  /// Whether the network is to blame: the service could not be reached, or the hub did not
  /// answer in time. That passes with the network, however often it happens.
  bool get isNetwork => switch (error) {
    TimeoutException() => true,
    RequestFailure(
      cause: RequestFailureCauseLibraryRegistrationFailed(:final kind),
    ) =>
      kind == RegisterLibraryFailureKind.remoteUnreachable,
    _ => false,
  };

  /// Whether only the user can make the next sign-in work: by allowing access, by using an
  /// account that has the service, by agreeing to the privacy notice, or by updating the
  /// system. Anything else may pass by itself, as a network that is down or an answer of the
  /// service that was not expected, so it is tried again later and the user is not told, until
  /// it has happened [signInFailuresBeforeTelling] times in a row without the network to blame.
  bool get userMustAct => repeated || _authorizationMustAct;

  bool get _authorizationMustAct => switch (authorization) {
    HostAuthorizationFailure.refused ||
    HostAuthorizationFailure.account ||
    HostAuthorizationFailure.privacy ||
    HostAuthorizationFailure.unsupported => true,
    HostAuthorizationFailure.noToken ||
    HostAuthorizationFailure.unavailable ||
    null => false,
  };

  /// Names the reason for the logs, which never hold a token.
  @override
  String toString() =>
      'DeviceSignInFailure(${authorization?.name ?? error.runtimeType})';
}

/// Remembers how the sign-ins of each library went, so that Rune does not sign in over and
/// over.
class DeviceSignInPolicy {
  DeviceSignInPolicy({DateTime Function()? clock})
    : _clock = clock ?? DateTime.now;

  final DateTime Function() _clock;
  final _signedIn = <String, DateTime>{};
  final _failed = <String, DateTime>{};
  final _failures = <String, DeviceSignInFailure>{};
  final _streaks = <String, int>{};

  bool _within(Map<String, DateTime> times, String recordId) {
    final time = times[recordId];
    return time != null && _clock().difference(time) < signInCooldown;
  }

  /// Whether a sign-in nobody asked for is worth trying: not when one failed within
  /// [signInCooldown], nor when one worked and the library needs a sign-in again all the same.
  bool worthTrying(String recordId) =>
      !_within(_signedIn, recordId) && !_within(_failed, recordId);

  void signedIn(String recordId) {
    _signedIn[recordId] = _clock();
    _failed.remove(recordId);
    _failures.remove(recordId);
    _streaks.remove(recordId);
  }

  /// Remembers that the sign-in of [recordId] failed, and returns the failure to go by. A
  /// failure that may pass by itself, with no network to blame, is counted: the
  /// [signInFailuresBeforeTelling]th in a row comes back as one the user must act on, and the
  /// count starts again. A failure of the network is not counted, whether [networkDown] says so
  /// or the failure itself does.
  DeviceSignInFailure failed(
    String recordId,
    DeviceSignInFailure failure, {
    bool networkDown = false,
  }) {
    if (!failure.userMustAct && !networkDown && !failure.isNetwork) {
      final streak = (_streaks[recordId] ?? 0) + 1;
      if (streak >= signInFailuresBeforeTelling) {
        _streaks.remove(recordId);
        failure = failure.asRepeated();
      } else {
        _streaks[recordId] = streak;
      }
    }
    _failed[recordId] = _clock();
    _failures[recordId] = failure;
    return failure;
  }

  /// Why the sign-in of [recordId] failed, while that is within [signInCooldown].
  DeviceSignInFailure? recentFailure(String recordId) =>
      _within(_failed, recordId) ? _failures[recordId] : null;

  /// The libraries whose last sign-in failed for a reason that may pass by itself, which are
  /// signed in again once the network is back.
  List<String> get awaitingRetry => [
    for (final MapEntry(:key, :value) in _failures.entries)
      if (!value.userMustAct) key,
  ];
}

/// Signs in the libraries whose sign-in lives on this device, which their records tell by
/// saying whether they are signed in, with what their platform host hands over.
class DeviceSignIns {
  DeviceSignIns(
    this.registry, {
    Map<String, HostedPlayerHost>? hosts,
    DeviceSignInPolicy? policy,
  }) : hosts = hosts ?? platformHosts,
       policy = policy ?? DeviceSignInPolicy();

  final LibraryRegistryProvider registry;
  final Map<String, HostedPlayerHost> hosts;
  final DeviceSignInPolicy policy;
  final _signIns = <String, Future<DeviceSignInFailure?>>{};
  final _flows = <String, Future<bool>>{};
  Future<void>? _startup;

  /// How the library of [recordId] signs in on this device. Null for a library that does not,
  /// and for one this device does not know.
  DeviceSignInMethod? methodOf(String recordId) {
    for (final record in registry.records) {
      if (record.recordId != recordId) continue;
      return record.signedIn == null ? null : deviceSignInMethod(record.kind);
    }
    return null;
  }

  /// The host that authorizes the library of [recordId], when this platform has it.
  AuthorizingHost? hostOf(String recordId) {
    final host = hosts[methodOf(recordId)?.host];
    return host is AuthorizingHost ? host : null;
  }

  /// Whether the library of [recordId] has a sign-in to make on this device.
  bool canSignIn(String recordId) => hostOf(recordId) != null;

  /// The sign-in flow of [recordId] under way, or the one [start] begins, shared by everyone
  /// who asks for it meanwhile.
  Future<bool> shared(String recordId, Future<bool> Function() start) =>
      _flows[recordId] ??= start().whenComplete(() {
        // A callback that returned the removed future would wait for itself.
        _flows.remove(recordId);
      });

  /// Signs the library of [recordId] in with what its host gives. Rune shows nothing of its
  /// own. With [prompt], the system asks the user for access where it was never asked. Null
  /// once it is signed in, else why not, which [policy] remembers. It never throws, and a
  /// sign-in already under way for the record is shared.
  Future<DeviceSignInFailure?> signIn(String recordId, {bool prompt = true}) {
    final host = hostOf(recordId);
    if (host == null) {
      return Future.value(
        const DeviceSignInFailure.authorization(
          HostAuthorizationFailure.unavailable,
        ),
      );
    }
    return _signIns[recordId] ??= _signIn(recordId, host, prompt).whenComplete(
      () {
        // A callback that returned the removed future would wait for itself.
        _signIns.remove(recordId);
      },
    );
  }

  Future<DeviceSignInFailure?> _signIn(
    String recordId,
    AuthorizingHost host,
    bool prompt,
  ) async {
    DeviceSignInFailure failed;
    var timedOut = false;
    try {
      var result = await host.authorize().timeout(
        silentAuthorizationTimeout,
        onTimeout: () {
          timedOut = true;
          return const HostAuthorizationFailed(
            HostAuthorizationFailure.unavailable,
          );
        },
      );
      if (prompt &&
          result is HostAuthorizationFailed &&
          result.failure == HostAuthorizationFailure.refused) {
        // Without access the system asks the user where it never did, and answers at once
        // where the user refused before. The user takes their time, so nothing ends the wait.
        result = await host.authorize(interactive: true);
      }
      switch (result) {
        case HostAuthorization():
          await registry
              .signInServiceLibrary(recordId, platformSignIn(result))
              .timeout(silentSignInTimeout);
          policy.signedIn(recordId);
          return null;
        case HostAuthorizationFailed(:final failure):
          logger.w('The host did not authorize a sign-in: ${failure.name}');
          failed = DeviceSignInFailure.authorization(failure);
      }
    } on RequestFailure catch (failure) {
      final cause = failure.cause;
      final kind = cause is RequestFailureCauseLibraryRegistrationFailed
          ? ' ${cause.kind.name}'
          : '';
      logger.w('The library did not sign in: ${cause.runtimeType}$kind');
      failed = DeviceSignInFailure.signIn(failure);
    } catch (error) {
      logger.w('The library did not sign in: ${error.runtimeType}');
      failed = DeviceSignInFailure.signIn(error);
    }
    return policy.failed(
      recordId,
      failed,
      networkDown: timedOut || _offline(host),
    );
  }

  static bool _offline(HostedPlayerHost host) =>
      host is NetworkReportingHost && host.online == false;

  /// Signs every library in once the registry has its records, as the hub forgets its sign-ins
  /// when Rune quits. It shows nothing and never throws, and a library that does not sign in
  /// is left to the sign-in flow. Only the library open last, which startup opens again, lets
  /// the system ask for access.
  Future<void> signInAtStartup() => _startup ??= _signInAll();

  Future<void> _signInAll() async {
    try {
      await registry.ready;
    } on Object {
      // Without its records the registry names no library to sign in.
      return;
    }
    await Future.wait([
      for (final record in registry.records)
        if (canSignIn(record.recordId))
          signIn(
            record.recordId,
            prompt: record.recordId == registry.lastOpenRecordId,
          ),
    ]);
  }

  /// Readies the library of [recordId], which startup opens again, once the sign-in startup
  /// made ended. The library opens signed in, and also when the sign-in failed for a reason
  /// that may pass by itself, as offline: it then shows what it kept and signs in by itself
  /// later. When only the user can make the sign-in work, the failure is thrown instead, so
  /// the library stays closed and startup tells why on the library chooser. A library that
  /// does not sign in on this device is ready as it is.
  Future<void> readyForStartup(String recordId) async {
    await signInAtStartup();
    final failure = policy.recentFailure(recordId);
    if (failure != null && failure.userMustAct) throw failure;
  }
}
