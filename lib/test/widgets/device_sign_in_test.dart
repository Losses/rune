import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/main.dart' show cafeMode;
import 'package:rune/providers/broadcast.dart';
import 'package:rune/providers/library_manager.dart';
import 'package:rune/providers/library_open_state.dart';
import 'package:rune/providers/library_registry.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/screens/settings_library/settings_library.dart';
import 'package:rune/utils/api/request_failure.dart';
import 'package:rune/utils/apple_music_sign_in.dart';
import 'package:rune/utils/device_sign_in.dart';
import 'package:rune/utils/dialogs/device_sign_in/device_sign_in_dialog.dart';
import 'package:rune/utils/hosted_player/apple_music_host.dart';
import 'package:rune/utils/hosted_player/hosted_player.dart';
import 'package:rune/utils/hosted_player/platform_hosts.dart';
import 'package:rune/utils/library_sign_in.dart';
import 'package:rune/utils/platform.dart';
import 'package:rune/utils/recover_failed_startup_open.dart';
import 'package:rune/widgets/router/rune_with_navigation_bar_and_playback_controllor.dart';

final _strings = lookupAppLocalizations(const Locale('en'));

const _signedInAppleRecord = LibraryRecordSummary(
  shareSync: false,
  recordId: 'apple-record',
  alias: 'Apple Music',
  kind: LibraryRecordKindSummaryAppleMusic(),
  signedIn: true,
);

const _appleRecord = LibraryRecordSummary(
  shareSync: false,
  recordId: 'apple-record',
  alias: 'Apple Music',
  kind: LibraryRecordKindSummaryAppleMusic(),
  signedIn: false,
);

const _localRecord = LibraryRecordSummary(
  shareSync: false,
  recordId: 'local-record',
  alias: 'Music',
  kind: LibraryRecordKindSummaryLocal(libraryId: 'library-1', path: '/music'),
);

const _subsonicRecord = LibraryRecordSummary(
  shareSync: false,
  recordId: 'subsonic-record',
  alias: 'Subsonic',
  kind: LibraryRecordKindSummarySubsonic(
    endpoint: 'https://music.example',
    account: '',
  ),
);

const _authorization = HostAuthorization(
  token: 'user-token',
  developerToken: 'developer-token',
);

const _refused = HostAuthorizationFailed(HostAuthorizationFailure.refused);
const _noToken = HostAuthorizationFailed(HostAuthorizationFailure.noToken);

/// A sign-in the hub refused for [kind].
RequestFailure _hubRefused(RegisterLibraryFailureKind kind) => RequestFailure(
  'SignInServiceLibraryRequest',
  RequestErrorKind.permissionDenied,
  'The sign-in failed',
  RequestFailureCauseLibraryRegistrationFailed(kind: kind),
);

class _Host implements AuthorizingHost {
  _Host(this.results);

  /// What each call answers, in order. The last one repeats.
  final List<HostAuthorizationResult> results;
  final calls = <bool>[];

  @override
  Future<HostAuthorizationResult> authorize({bool interactive = false}) async {
    calls.add(interactive);
    return results[(calls.length - 1).clamp(0, results.length - 1)];
  }

  @override
  Stream<void> get accessLost => const Stream.empty();

  @override
  void handle(
    Map<String, dynamic> command,
    void Function(Map<String, dynamic>) emit,
  ) {}
}

/// A host that reports that the device has no network.
class _OfflineHost extends _Host implements NetworkReportingHost {
  _OfflineHost(super.results);

  @override
  bool? get online => false;

  @override
  Stream<void> get networkBack => const Stream.empty();
}

/// Answers silently with [silent], and waits for [interactive] when asked to
/// show something.
class _GatedHost implements AuthorizingHost {
  _GatedHost({required this.silent});

  final HostAuthorizationResult silent;
  final interactive = Completer<HostAuthorizationResult>();
  final calls = <bool>[];

  @override
  Future<HostAuthorizationResult> authorize({bool interactive = false}) {
    calls.add(interactive);
    return interactive ? this.interactive.future : Future.value(silent);
  }

  @override
  Stream<void> get accessLost => const Stream.empty();

  @override
  void handle(
    Map<String, dynamic> command,
    void Function(Map<String, dynamic>) emit,
  ) {}
}

class _StuckHost implements AuthorizingHost {
  @override
  Future<HostAuthorizationResult> authorize({bool interactive = false}) =>
      Completer<HostAuthorizationResult>().future;

  @override
  Stream<void> get accessLost => const Stream.empty();

  @override
  void handle(
    Map<String, dynamic> command,
    void Function(Map<String, dynamic>) emit,
  ) {}
}

/// A hub that opens an Apple Music library whether or not it is signed in, as
/// the real one does: one that is not opens from what it kept. Its records say
/// the Apple Music library is not signed in, as the real hub's do at startup.
class _Hub {
  _Hub({
    this.records = const [_appleRecord, _localRecord],
    this.open,
    this.lastOpen,
    Future<void>? listed,
  }) : _listed = listed;

  final List<LibraryRecordSummary> records;
  final String? open;
  final String? lastOpen;
  final Future<void>? _listed;
  bool signedIn = false;
  Object? signInFailure;

  /// Holds the hub's answer to a sign-in until it completes, when set.
  Future<void>? signInGate;
  final signIns = <String>[];
  final opens = <String>[];
  final removed = <String>[];

  late final registry = LibraryRegistryProvider(
    loadRecords: () async {
      await _listed;
      return ListLibraryRecordsResponse(
        records: records,
        invalidRecords: const [],
        open: open == null
            ? null
            : OpenRecord(
                recordId: open!,
                generation: Uint64.fromBigInt(BigInt.one),
              ),
        lastOpenRecord: lastOpen,
      );
    },
    openRecord: (recordId, callId) async {
      opens.add(recordId);
      return OpenLibraryResponse(
        callId: Uint64.fromBigInt(BigInt.from(callId)),
        recordId: recordId,
        generation: Uint64.fromBigInt(BigInt.one),
      );
    },
    signInService: (recordId, signIn) async {
      final platform = signIn as ServiceSignInPlatform;
      signIns.add('$recordId ${platform.userToken} ${platform.developerToken}');
      await signInGate;
      if (signInFailure case final failure?) throw failure;
      signedIn = true;
    },
    removeRecord: (recordId) async {
      removed.add(recordId);
      return const RemoveLibraryRecordResponse();
    },
  );

  /// The sign-ins of this hub's libraries through [host], when there is one.
  DeviceSignIns signInsWith(
    HostedPlayerHost? host, {
    DeviceSignInPolicy? policy,
  }) => DeviceSignIns(
    registry,
    methods: platformSignInMethods,
    hosts: {AppleMusicHost.family: ?host},
    policy: policy,
  );
}

Future<_Hub> _hub({
  List<LibraryRecordSummary> records = const [_appleRecord, _localRecord],
  String? open,
}) async {
  final hub = _Hub(records: records, open: open);
  addTearDown(hub.registry.dispose);
  await hub.registry.ready;
  return hub;
}

Widget _app(Widget home) => FluentApp(
  localizationsDelegates: AppLocalizations.localizationsDelegates,
  supportedLocales: AppLocalizations.supportedLocales,
  home: home,
);

/// A context to start flows from, under an app that can show their dialogs.
Future<BuildContext Function()> _context(WidgetTester tester) async {
  late BuildContext captured;
  await tester.pumpWidget(
    _app(
      Builder(
        builder: (context) {
          captured = context;
          return const SizedBox();
        },
      ),
    ),
  );
  return () => captured;
}

/// Opens the Apple Music library from a button, recording what the open threw.
Future<Object? Function()> _open(
  WidgetTester tester,
  _Hub hub,
  DeviceSignIns signIns,
) async {
  Object? thrown;
  await tester.pumpWidget(
    Provider<DeviceSignIns>.value(
      value: signIns,
      child: _app(
        Builder(
          builder: (context) => FilledButton(
            onPressed: () async {
              try {
                await openLibraryWithSignIn(
                  context,
                  hub.registry,
                  _appleRecord.recordId,
                );
              } catch (error) {
                thrown = error;
              }
            },
            child: const Text('Open'),
          ),
        ),
      ),
    ),
  );
  await tester.tap(find.text('Open'));
  await tester.pumpAndSettle();
  return () => thrown;
}

void main() {
  group('the message for a host that gave no sign-in', () {
    for (final (platform, failure, message) in [
      (
        TargetPlatform.macOS,
        HostAuthorizationFailure.unsupported,
        _strings.appleMusicNeedsNewerMacOS,
      ),
      (
        TargetPlatform.iOS,
        HostAuthorizationFailure.unsupported,
        _strings.appleMusicNeedsNewerIOS,
      ),
      (
        TargetPlatform.macOS,
        HostAuthorizationFailure.refused,
        _strings.appleMusicAccessDenied,
      ),
      (
        TargetPlatform.iOS,
        HostAuthorizationFailure.refused,
        _strings.appleMusicAccessDeniedIOS,
      ),
      (
        TargetPlatform.macOS,
        HostAuthorizationFailure.account,
        _strings.appleMusicAccountUnavailable,
      ),
      (
        TargetPlatform.iOS,
        HostAuthorizationFailure.account,
        _strings.appleMusicAccountUnavailableIOS,
      ),
      (
        TargetPlatform.macOS,
        HostAuthorizationFailure.privacy,
        _strings.appleMusicPrivacyNotice,
      ),
      (
        TargetPlatform.iOS,
        HostAuthorizationFailure.privacy,
        _strings.appleMusicPrivacyNotice,
      ),
    ]) {
      test(
        'names the system Rune runs on: ${failure.name} on ${platform.name}',
        () {
          debugDefaultTargetPlatformOverride = platform;
          addTearDown(() => debugDefaultTargetPlatformOverride = null);
          expect(appleMusicAuthorizationMessage(_strings, failure), message);
        },
      );
    }
  });

  group('the library subtitle', () {
    for (final (platform, text) in [
      (TargetPlatform.macOS, _strings.appleMusicSubtitle),
      (TargetPlatform.iOS, _strings.appleMusicSubtitleIOS),
    ]) {
      test('says where it plays on ${platform.name}', () {
        debugDefaultTargetPlatformOverride = platform;
        addTearDown(() => debugDefaultTargetPlatformOverride = null);
        expect(appleMusicSubtitleText(_strings), text);
      });
    }

    test('an iOS build on a Mac speaks of the Mac', () {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      debugIosDevice = IosDevice.mac;
      addTearDown(() {
        debugDefaultTargetPlatformOverride = null;
        debugIosDevice = null;
      });
      expect(appleMusicSubtitleText(_strings), _strings.appleMusicSubtitle);
      expect(
        appleMusicAuthorizationMessage(
          _strings,
          HostAuthorizationFailure.refused,
        ),
        _strings.appleMusicAccessDenied,
      );
      expect(
        appleMusicAuthorizationMessage(
          _strings,
          HostAuthorizationFailure.account,
        ),
        _strings.appleMusicAccountUnavailable,
      );
    });
  });

  group('which libraries sign in on this device', () {
    test('the ones that say whether they are signed in', () async {
      final hub = await _hub(
        records: const [_appleRecord, _localRecord, _subsonicRecord],
      );
      final signIns = hub.signInsWith(_Host([_authorization]));

      expect(signIns.methodOf('apple-record'), appleMusicSignInMethod);
      expect(signIns.canSignIn('apple-record'), isTrue);
      for (final recordId in [
        'local-record',
        'subsonic-record',
        'missing-record',
      ]) {
        expect(signIns.methodOf(recordId), isNull, reason: recordId);
        expect(signIns.canSignIn(recordId), isFalse, reason: recordId);
      }
    });

    test('none without the host of their kind', () async {
      final hub = await _hub();
      final signIns = hub.signInsWith(null);

      expect(signIns.methodOf('apple-record'), appleMusicSignInMethod);
      expect(signIns.canSignIn('apple-record'), isFalse);
      final failure = await signIns.signIn('apple-record');
      expect(failure?.authorization, HostAuthorizationFailure.unavailable);
      expect(hub.signIns, isEmpty);
    });
  });

  group('which failures the user must act on', () {
    for (final (failure, mustAct) in [
      (HostAuthorizationFailure.refused, true),
      (HostAuthorizationFailure.account, true),
      (HostAuthorizationFailure.privacy, true),
      (HostAuthorizationFailure.unsupported, true),
      (HostAuthorizationFailure.noToken, false),
      (HostAuthorizationFailure.unavailable, false),
    ]) {
      test('a host that answers ${failure.name}', () {
        expect(DeviceSignInFailure.authorization(failure).userMustAct, mustAct);
      });
    }

    for (final (kind, message) in [
      (
        RegisterLibraryFailureKind.remoteUnreachable,
        _strings.appleMusicUnreachable,
      ),
      (
        RegisterLibraryFailureKind.remoteProtocol,
        _strings.appleMusicUnexpectedResponse,
      ),
      (
        RegisterLibraryFailureKind.remoteNotAuthorized,
        _strings.appleMusicSignInFailed,
      ),
      (RegisterLibraryFailureKind.invalidSource, _strings.appleMusicNoToken),
    ]) {
      test('a hub that answers ${kind.name} may pass by itself', () {
        final failure = DeviceSignInFailure.signIn(_hubRefused(kind));
        expect(failure.userMustAct, isFalse);
        expect(appleMusicSignInMethod.message(_strings, failure), message);
      });
    }

    test('a hub that does not answer in time may pass by itself', () {
      final failure = DeviceSignInFailure.signIn(TimeoutException('late'));
      expect(failure.userMustAct, isFalse);
      expect(
        appleMusicSignInMethod.message(_strings, failure),
        _strings.appleMusicUnreachable,
      );
    });
  });

  group('opening a library that is not signed in on this device', () {
    testWidgets('signs in silently and opens, without a dialog', (
      tester,
    ) async {
      final hub = await _hub();
      final host = _Host([_authorization]);
      final thrown = await _open(tester, hub, hub.signInsWith(host));

      expect(host.calls, [false]);
      expect(hub.signIns, ['apple-record user-token developer-token']);
      // It signs in first, then opens once.
      expect(hub.opens, ['apple-record']);
      expect(hub.registry.openRecordId, 'apple-record');
      expect(find.byType(ContentDialog), findsNothing);
      expect(thrown(), isNull);
    });

    testWidgets('a library that says it is signed in opens as it is', (
      tester,
    ) async {
      final hub = await _hub(records: const [_signedInAppleRecord]);
      final host = _Host([_authorization]);
      final thrown = await _open(tester, hub, hub.signInsWith(host));

      expect(host.calls, isEmpty);
      expect(hub.signIns, isEmpty);
      expect(hub.opens, ['apple-record']);
      expect(thrown(), isNull);
    });

    testWidgets(
      'where access was never asked, the system asks and Rune shows nothing',
      (tester) async {
        final hub = await _hub();
        // Silently there is no access. Asked, the system gets it from the user.
        final host = _Host([_refused, _authorization]);
        final thrown = await _open(tester, hub, hub.signInsWith(host));

        expect(host.calls, [false, true]);
        expect(hub.signIns, ['apple-record user-token developer-token']);
        expect(hub.registry.openRecordId, 'apple-record');
        expect(find.byType(ContentDialog), findsNothing);
        expect(thrown(), isNull);
      },
    );

    testWidgets('refused access is told, and Close leaves it closed', (
      tester,
    ) async {
      final hub = await _hub();
      final host = _Host([_refused]);
      final thrown = await _open(tester, hub, hub.signInsWith(host));

      expect(host.calls, [false, true]);
      expect(hub.signIns, isEmpty);
      expect(find.text(_strings.loginFailed), findsOneWidget);
      expect(find.text(_strings.appleMusicAccessDenied), findsOneWidget);
      expect(
        find.widgetWithText(FilledButton, _strings.playOnRetry),
        findsOneWidget,
      );
      await tester.tap(find.widgetWithText(Button, _strings.close));
      await tester.pumpAndSettle();

      expect(find.byType(ContentDialog), findsNothing);
      expect(host.calls, [false, true]);
      expect(hub.opens, isEmpty);
      expect(hub.registry.openRecordId, isNull);
      expect(thrown(), isNull);
    });

    testWidgets('Retry that signs in opens the library', (tester) async {
      final hub = await _hub();
      final host = _Host([_refused, _refused, _authorization]);
      final thrown = await _open(tester, hub, hub.signInsWith(host));

      await tester.tap(find.text(_strings.playOnRetry));
      await tester.pumpAndSettle();

      expect(host.calls, [false, true, false]);
      expect(hub.signIns, ['apple-record user-token developer-token']);
      expect(hub.registry.openRecordId, 'apple-record');
      expect(find.byType(ContentDialog), findsNothing);
      expect(thrown(), isNull);
    });

    testWidgets('Retry that fails again tells the new reason', (tester) async {
      final hub = await _hub();
      final host = _Host([
        _refused,
        _refused,
        const HostAuthorizationFailed(HostAuthorizationFailure.account),
      ]);
      await _open(tester, hub, hub.signInsWith(host));
      expect(find.text(_strings.appleMusicAccessDenied), findsOneWidget);

      await tester.tap(find.text(_strings.playOnRetry));
      await tester.pumpAndSettle();

      expect(host.calls, [false, true, false]);
      expect(find.text(_strings.appleMusicAccessDenied), findsNothing);
      expect(find.text(_strings.appleMusicAccountUnavailable), findsOneWidget);
      expect(hub.registry.openRecordId, isNull);
      await tester.tap(find.text(_strings.close));
      await tester.pumpAndSettle();
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets(
      'a sign-in the service refuses after a Retry is told without its tokens',
      (tester) async {
        final hub = await _hub()
          ..signInFailure = _hubRefused(
            RegisterLibraryFailureKind.remoteNotAuthorized,
          );
        final host = _Host([_refused, _refused, _authorization]);
        final thrown = await _open(tester, hub, hub.signInsWith(host));
        expect(find.text(_strings.appleMusicAccessDenied), findsOneWidget);

        await tester.tap(find.text(_strings.playOnRetry));
        await tester.pumpAndSettle();
        expect(hub.signIns, hasLength(1));
        expect(find.text(_strings.loginFailed), findsOneWidget);
        expect(find.text(_strings.appleMusicSignInFailed), findsOneWidget);
        expect(find.textContaining('user-token'), findsNothing);
        await tester.tap(find.text(_strings.close));
        await tester.pumpAndSettle();

        expect(host.calls, [false, true, false]);
        expect(hub.registry.openRecordId, isNull);
        expect(thrown(), isNull);
      },
    );

    for (final (reason, failure, answer) in [
      (
        'the service cannot be reached',
        _hubRefused(RegisterLibraryFailureKind.remoteUnreachable),
        _authorization,
      ),
      ('the host has no token yet', null, _noToken),
    ]) {
      testWidgets(
        'a sign-in that fails because $reason opens the library from what it kept, '
        'with no dialog',
        (tester) async {
          final hub = await _hub()
            ..signInFailure = failure;
          final host = _Host([answer]);
          final signIns = hub.signInsWith(host);
          final thrown = await _open(tester, hub, signIns);

          expect(host.calls, [false]);
          expect(hub.opens, ['apple-record']);
          expect(hub.registry.openRecordId, 'apple-record');
          expect(find.byType(ContentDialog), findsNothing);
          expect(thrown(), isNull);
          // It is signed in again once the network is back.
          expect(signIns.policy.awaitingRetry, ['apple-record']);
        },
      );
    }

    testWidgets(
      'without a host it opens without a sign-in, and shows nothing',
      (tester) async {
        final hub = await _hub();
        final thrown = await _open(tester, hub, hub.signInsWith(null));

        expect(find.byType(ContentDialog), findsNothing);
        expect(thrown(), isNull);
        expect(hub.signIns, isEmpty);
        expect(hub.opens, ['apple-record']);
      },
    );

    testWidgets('a library that keeps its credential opens as before', (
      tester,
    ) async {
      final registry = LibraryRegistryProvider(
        loadRecords: () async => const ListLibraryRecordsResponse(
          records: [_subsonicRecord],
          invalidRecords: [],
          lastOpenRecord: null,
        ),
        openRecord: (_, _) async => throw const RequestFailure(
          'OpenLibraryRequest',
          RequestErrorKind.permissionDenied,
          'The Subsonic credentials were rejected',
          RequestFailureCauseLibraryOpenFailed(
            kind: OpenLibraryFailureKind.remoteNotAuthorized,
          ),
        ),
      );
      addTearDown(registry.dispose);
      await registry.ready;
      final contextOf = await _context(tester);
      Object? thrown;
      unawaited(
        openLibraryWithSignIn(
          contextOf(),
          registry,
          _subsonicRecord.recordId,
        ).then((_) {}, onError: (Object error) => thrown = error),
      );
      await tester.pumpAndSettle();
      // No sign-ins of this device are looked up for it.
      expect(find.text(_strings.retryLogin), findsOneWidget);
      await tester.tap(find.text(_strings.cancel));
      await tester.pumpAndSettle();

      expect(thrown, isA<RequestFailure>());
    });
  });

  group('a sign-in Rune makes by itself', () {
    test(
      'startup signs in every library that signs in here, and only those',
      () async {
        final hub = await _hub(
          records: const [_appleRecord, _localRecord, _subsonicRecord],
        );
        final host = _Host([_authorization]);

        await hub.signInsWith(host).signInAtStartup();

        expect(host.calls, [false]);
        expect(hub.signIns, ['apple-record user-token developer-token']);
      },
    );

    test('startup sends nothing without a host or an authorization', () async {
      final hub = await _hub();

      await hub.signInsWith(null).signInAtStartup();
      final host = _Host([_refused]);
      await hub.signInsWith(host).signInAtStartup();

      // A library startup does not open again has the system ask nothing.
      expect(host.calls, [false]);
      expect(hub.signIns, isEmpty);
    });

    test(
      'startup lets the system ask for access to the library open last',
      () async {
        final hub = _Hub(lastOpen: 'apple-record');
        addTearDown(hub.registry.dispose);
        final host = _Host([_refused, _authorization]);

        await hub.signInsWith(host).signInAtStartup();

        expect(host.calls, [false, true]);
        expect(hub.signIns, ['apple-record user-token developer-token']);
      },
    );

    test('startup signs in once, however often it is asked', () async {
      final hub = await _hub();
      final host = _Host([_authorization]);
      final signIns = hub.signInsWith(host);

      await Future.wait([signIns.signInAtStartup(), signIns.signInAtStartup()]);
      await signIns.readyForStartup('apple-record');

      expect(host.calls, [false]);
    });

    test('a sign-in that fails is swallowed and told as its reason', () async {
      final hub = await _hub()
        ..signInFailure = StateError('offline');
      final host = _Host([_authorization]);
      final signIns = hub.signInsWith(host);

      await signIns.signInAtStartup();
      final failure = await signIns.signIn('apple-record');

      expect(failure?.userMustAct, isFalse);
      expect(hub.signIns, hasLength(2));
      expect(signIns.policy.awaitingRetry, ['apple-record']);
      expect(signIns.policy.worthTrying('apple-record'), isFalse);
    });

    test('sign-ins of one library under way are shared', () async {
      final hub = await _hub();
      final host = _Host([_authorization]);
      final signIns = hub.signInsWith(host);

      final results = await Future.wait([
        signIns.signIn('apple-record'),
        signIns.signIn('apple-record'),
      ]);

      expect(results, [null, null]);
      expect(host.calls, [false]);
      expect(hub.signIns, hasLength(1));
    });

    testWidgets('a silent authorization that never answers fails in time', (
      tester,
    ) async {
      final hub = await _hub();
      DeviceSignInFailure? result;
      unawaited(
        hub
            .signInsWith(_StuckHost())
            .signIn('apple-record')
            .then((value) => result = value),
      );
      await tester.pump(const Duration(seconds: 9));
      expect(result, isNull);
      await tester.pump(const Duration(seconds: 2));
      expect(result?.authorization, HostAuthorizationFailure.unavailable);
      expect(result?.userMustAct, isFalse);
      expect(hub.signIns, isEmpty);
    });

    testWidgets('a sign-in the hub never answers fails in time', (
      tester,
    ) async {
      final hub = await _hub()
        ..signInGate = Completer<void>().future;
      DeviceSignInFailure? result;
      unawaited(
        hub
            .signInsWith(_Host([_authorization]))
            .signIn('apple-record')
            .then((value) => result = value),
      );
      await tester.pump(const Duration(seconds: 14));
      expect(hub.signIns, hasLength(1));
      expect(result, isNull);
      await tester.pump(const Duration(seconds: 2));
      expect(result?.error, isA<TimeoutException>());
      expect(result?.userMustAct, isFalse);
    });
  });

  group('startup', () {
    test('signing in waits for the registry to list its records', () async {
      final listed = Completer<void>();
      final hub = _Hub(listed: listed.future);
      addTearDown(hub.registry.dispose);
      final host = _Host([_authorization]);

      final signedIn = hub.signInsWith(host).signInAtStartup();
      await pumpEventQueue();
      expect(host.calls, isEmpty);
      listed.complete();
      await signedIn;

      expect(host.calls, [false]);
      expect(hub.signIns, ['apple-record user-token developer-token']);
    });

    test(
      'startup tells it is still opening while it waits for a sign-in',
      () async {
        final hub = _Hub(lastOpen: 'apple-record');
        addTearDown(hub.registry.dispose);
        final signedIn = Completer<void>();
        final state = LibraryOpenStateProvider(
          null,
          hub.registry,
          beforeOpen: (_) => signedIn.future,
        );
        addTearDown(state.dispose);

        await pumpEventQueue();
        expect(state.isOpeningAtStartup, isTrue);
        expect(hub.opens, isEmpty);
        signedIn.complete();
        expect(await state.initialOpen, isTrue);
        expect(state.isOpeningAtStartup, isFalse);
      },
    );

    test('startup that opens nothing is not opening for long', () async {
      final hub = _Hub();
      addTearDown(hub.registry.dispose);
      final state = LibraryOpenStateProvider(null, hub.registry);
      addTearDown(state.dispose);

      expect(await state.initialOpen, isFalse);
      expect(state.isOpeningAtStartup, isFalse);
    });

    test('the last library opens once it is signed in', () async {
      final hub = _Hub(lastOpen: 'apple-record');
      addTearDown(hub.registry.dispose);
      final signIns = hub.signInsWith(_Host([_authorization]));
      unawaited(signIns.signInAtStartup());
      final state = LibraryOpenStateProvider(
        null,
        hub.registry,
        beforeOpen: signIns.readyForStartup,
      );
      addTearDown(state.dispose);

      expect(await state.initialOpen, isTrue);
      expect(hub.signIns, hasLength(1));
      expect(hub.opens, ['apple-record']);
      expect(state.initialOpenFailure, isNull);
    });

    test('a last library that does not sign in here opens as it is', () async {
      final hub = _Hub(lastOpen: 'local-record');
      addTearDown(hub.registry.dispose);
      final signIns = hub.signInsWith(_Host([_refused]));
      final state = LibraryOpenStateProvider(
        null,
        hub.registry,
        beforeOpen: signIns.readyForStartup,
      );
      addTearDown(state.dispose);

      expect(await state.initialOpen, isTrue);
      expect(hub.opens, ['local-record']);
    });

    /// Starts as the app does with an Apple Music library open last: signs in,
    /// opens it unless only the user can make the sign-in work, and recovers a
    /// failed open on the library chooser. Tells what happened in order.
    Future<List<String>> start(
      WidgetTester tester,
      _Hub hub,
      DeviceSignIns signIns,
    ) async {
      final events = <String>[];
      final contextOf = await _context(tester);
      late LibraryOpenStateProvider state;
      late bool opened;
      await tester.runAsync(() async {
        unawaited(signIns.signInAtStartup());
        state = LibraryOpenStateProvider(
          null,
          hub.registry,
          beforeOpen: signIns.readyForStartup,
        );
        opened = await state.initialOpen;
      });
      addTearDown(state.dispose);
      unawaited(
        recoverFailedStartupOpen(
          libraryOpened: Future.value(opened),
          failure: () => state.initialOpenFailure,
          libraryIsOpen: () => state.hasOpenLibrary,
          signInAgain: () async => false,
          showChooser: (failure) {
            events.add('chooser');
            if (failure is! DeviceSignInFailure) return;
            unawaited(
              reopenAfterDeviceSignIn(
                contextOf(),
                signIns,
                'apple-record',
                failure,
                showLibrary: () => events.add('library'),
              ).then((_) => events.add('done')),
            );
          },
        ),
      );
      await tester.pumpAndSettle();
      return events;
    }

    testWidgets(
      'a failure the user must act on does not open it: the chooser tells why',
      (tester) async {
        final hub = _Hub(lastOpen: 'apple-record');
        addTearDown(hub.registry.dispose);
        final host = _Host([_refused, _refused, _authorization]);
        final events = await start(tester, hub, hub.signInsWith(host));

        // The sign-in made at startup is the one told. No other was made for
        // the dialog, and the library was not opened.
        expect(host.calls, [false, true]);
        expect(events, ['chooser']);
        expect(hub.opens, isEmpty);
        expect(hub.registry.openRecordId, isNull);
        expect(find.text(_strings.loginFailed), findsOneWidget);
        expect(find.text(_strings.appleMusicAccessDenied), findsOneWidget);

        await tester.tap(find.text(_strings.playOnRetry));
        await tester.pumpAndSettle();
        expect(host.calls, [false, true, false]);
        expect(events, ['chooser', 'library', 'done']);
        expect(hub.registry.openRecordId, 'apple-record');
        expect(find.byType(ContentDialog), findsNothing);
      },
      variant: TargetPlatformVariant.only(TargetPlatform.macOS),
    );

    testWidgets('Close stays on the chooser, with nothing more to say', (
      tester,
    ) async {
      final hub = _Hub(lastOpen: 'apple-record');
      addTearDown(hub.registry.dispose);
      final host = _Host([
        const HostAuthorizationFailed(HostAuthorizationFailure.privacy),
      ]);
      final events = await start(tester, hub, hub.signInsWith(host));

      expect(find.text(_strings.appleMusicPrivacyNotice), findsOneWidget);
      await tester.tap(find.text(_strings.close));
      await tester.pumpAndSettle();

      expect(host.calls, [false]);
      expect(events, ['chooser', 'done']);
      expect(hub.registry.openRecordId, isNull);
      expect(find.byType(ContentDialog), findsNothing);
    });

    testWidgets(
      'an offline cold start opens the library with no dialog, and signs it in once the '
      'network is back',
      (tester) async {
        final hub = _Hub(lastOpen: 'apple-record')
          ..signInFailure = _hubRefused(
            RegisterLibraryFailureKind.remoteUnreachable,
          );
        addTearDown(hub.registry.dispose);
        final signIns = hub.signInsWith(_Host([_authorization]));
        final events = await start(tester, hub, signIns);

        // It opens from what it kept, and nothing is told.
        expect(events, isEmpty);
        expect(hub.opens, ['apple-record']);
        expect(hub.registry.openRecordId, 'apple-record');
        expect(hub.signedIn, isFalse);
        expect(find.byType(ContentDialog), findsNothing);
        expect(signIns.policy.awaitingRetry, ['apple-record']);

        // The network comes back, and the library signs in by itself, showing
        // nothing.
        hub.signInFailure = null;
        final contextOf = await _context(tester);
        AutomaticDeviceSignIn(
          signIns: signIns,
          dialogContext: contextOf,
          leaveLibrary: () async => events.add('chooser'),
          showLibrary: () => events.add('library'),
        ).networkBack();
        await tester.pumpAndSettle();
        expect(hub.signIns, hasLength(2));
        expect(hub.signedIn, isTrue);
        expect(signIns.policy.awaitingRetry, isEmpty);
        expect(events, isEmpty);
        expect(hub.registry.openRecordId, 'apple-record');
        expect(find.byType(ContentDialog), findsNothing);
      },
    );
  });

  group('a library that turns out to need a sign-in', () {
    late DateTime now;
    late DeviceSignInPolicy policy;
    late List<String> events;

    setUp(() {
      now = DateTime(2026, 11, 15, 12);
      policy = DeviceSignInPolicy(clock: () => now);
      events = [];
    });

    AutomaticDeviceSignIn automaticFor(
      _Hub hub,
      BuildContext Function() contextOf,
      HostedPlayerHost? host,
    ) => AutomaticDeviceSignIn(
      signIns: hub.signInsWith(host, policy: policy),
      dialogContext: contextOf,
      leaveLibrary: () async {
        events.add('chooser');
        hub.registry.onClosed();
      },
      showLibrary: () => events.add('library'),
    );

    testWidgets('signs in by itself and shows nothing', (tester) async {
      final hub = await _hub(open: 'apple-record');
      final contextOf = await _context(tester);
      final host = _Host([_authorization]);
      final automatic = automaticFor(hub, contextOf, host);

      expect(automatic.needsSignIn('apple-record'), isTrue);
      await tester.pumpAndSettle();

      expect(host.calls, [false]);
      expect(hub.signIns, hasLength(1));
      expect(events, isEmpty);
      expect(find.byType(ContentDialog), findsNothing);
    });

    testWidgets(
      'where access was never asked the system asks, and failures meanwhile are left to it',
      (tester) async {
        final hub = await _hub(open: 'apple-record');
        final contextOf = await _context(tester);
        final host = _GatedHost(silent: _refused);
        final automatic = automaticFor(hub, contextOf, host);

        for (var index = 0; index < 3; index++) {
          expect(automatic.needsSignIn('apple-record'), isTrue);
          automatic.accessLost(AppleMusicHost.family);
          await tester.pump();
        }
        // The system's prompt is up, and Rune shows nothing of its own.
        expect(host.calls, [false, true]);
        expect(find.byType(ContentDialog), findsNothing);

        host.interactive.complete(_authorization);
        await tester.pumpAndSettle();
        expect(hub.signIns, hasLength(1));
        expect(events, isEmpty);
        expect(hub.registry.openRecordId, 'apple-record');
        expect(find.byType(ContentDialog), findsNothing);
      },
    );

    testWidgets('a library that does not sign in here is left to others', (
      tester,
    ) async {
      final hub = await _hub(
        records: const [_appleRecord, _localRecord, _subsonicRecord],
        open: 'apple-record',
      );
      final contextOf = await _context(tester);
      final host = _Host([_authorization]);
      final automatic = automaticFor(hub, contextOf, host);

      for (final recordId in [
        'local-record',
        'subsonic-record',
        'missing-record',
      ]) {
        expect(automatic.needsSignIn(recordId), isFalse, reason: recordId);
      }
      await tester.pumpAndSettle();

      expect(host.calls, isEmpty);
      expect(events, isEmpty);
      expect(find.byType(ContentDialog), findsNothing);
    });

    testWidgets('without a host the failure is taken and nothing is shown', (
      tester,
    ) async {
      final hub = await _hub(open: 'apple-record');
      final contextOf = await _context(tester);
      final automatic = automaticFor(hub, contextOf, null);

      expect(automatic.needsSignIn('apple-record'), isTrue);
      automatic.accessLost(AppleMusicHost.family);
      await tester.pumpAndSettle();

      expect(hub.signIns, isEmpty);
      expect(events, isEmpty);
      expect(find.byType(ContentDialog), findsNothing);
    });

    testWidgets(
      'lost access signs in the open library only when it signs in through that host',
      (tester) async {
        const second = LibraryRecordSummary(
          shareSync: false,
          recordId: 'apple-second',
          alias: 'Apple Music 2',
          kind: LibraryRecordKindSummaryAppleMusic(),
          signedIn: true,
        );
        for (final (open, family, expected) in [
          (
            'apple-second',
            AppleMusicHost.family,
            ['apple-second user-token developer-token'],
          ),
          (
            'apple-record',
            AppleMusicHost.family,
            ['apple-record user-token developer-token'],
          ),
          ('apple-record', 'otherHost', <String>[]),
          ('local-record', AppleMusicHost.family, <String>[]),
          (null, AppleMusicHost.family, <String>[]),
        ]) {
          final hub = await _hub(
            records: const [_appleRecord, second, _localRecord],
            open: open,
          );
          final contextOf = await _context(tester);
          automaticFor(
            hub,
            contextOf,
            _Host([_authorization]),
          ).accessLost(family);
          await tester.pumpAndSettle();
          expect(hub.signIns, expected, reason: 'open: $open, host: $family');
          expect(find.byType(ContentDialog), findsNothing);
          // Each round starts with a policy of its own.
          policy = DeviceSignInPolicy(clock: () => now);
        }
      },
    );

    for (final (answers, message) in [
      ([_refused], _strings.appleMusicAccessDenied),
      (
        [const HostAuthorizationFailed(HostAuthorizationFailure.account)],
        _strings.appleMusicAccountUnavailable,
      ),
      (
        [const HostAuthorizationFailed(HostAuthorizationFailure.privacy)],
        _strings.appleMusicPrivacyNotice,
      ),
      (
        [const HostAuthorizationFailed(HostAuthorizationFailure.unsupported)],
        _strings.appleMusicNeedsNewerMacOS,
      ),
    ]) {
      testWidgets(
        'a failure the user must act on closes the library and tells why on the chooser: '
        '${answers.single.failure.name}',
        (tester) async {
          final hub = await _hub(open: 'apple-record');
          final contextOf = await _context(tester);
          final automatic = automaticFor(hub, contextOf, _Host(answers));

          automatic.needsSignIn('apple-record');
          await tester.pumpAndSettle();

          expect(events, ['chooser']);
          expect(hub.registry.openRecordId, isNull);
          expect(find.text(_strings.loginFailed), findsOneWidget);
          expect(find.text(message), findsOneWidget);
          expect(
            find.widgetWithText(FilledButton, _strings.playOnRetry),
            findsOneWidget,
          );

          // Close stays on the chooser.
          await tester.tap(find.widgetWithText(Button, _strings.close));
          await tester.pumpAndSettle();
          expect(events, ['chooser']);
          expect(hub.registry.openRecordId, isNull);
          expect(find.byType(ContentDialog), findsNothing);
        },
        variant: TargetPlatformVariant.only(TargetPlatform.macOS),
      );
    }

    testWidgets('lost access that stays refused does the same', (tester) async {
      final hub = await _hub(open: 'apple-record');
      final contextOf = await _context(tester);
      final host = _Host([_refused]);
      automaticFor(hub, contextOf, host).accessLost(AppleMusicHost.family);
      await tester.pumpAndSettle();

      // The system was asked, and had been refused before.
      expect(host.calls, [false, true]);
      expect(events, ['chooser']);
      expect(find.text(_strings.appleMusicAccessDenied), findsOneWidget);
      await tester.tap(find.text(_strings.close));
      await tester.pumpAndSettle();
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('Retry that signs in opens the library again', (tester) async {
      final hub = await _hub(open: 'apple-record');
      final contextOf = await _context(tester);
      final host = _Host([
        _refused,
        _refused,
        const HostAuthorizationFailed(HostAuthorizationFailure.account),
        _authorization,
      ]);
      final automatic = automaticFor(hub, contextOf, host);

      automatic.needsSignIn('apple-record');
      await tester.pumpAndSettle();
      expect(find.text(_strings.appleMusicAccessDenied), findsOneWidget);

      // A Retry that fails tells the new reason and stays.
      await tester.tap(find.text(_strings.playOnRetry));
      await tester.pumpAndSettle();
      expect(find.text(_strings.appleMusicAccessDenied), findsNothing);
      expect(find.text(_strings.appleMusicAccountUnavailable), findsOneWidget);
      expect(events, ['chooser']);

      await tester.tap(find.text(_strings.playOnRetry));
      await tester.pumpAndSettle();
      expect(host.calls, [false, true, false, false]);
      expect(hub.signIns, hasLength(1));
      expect(events, ['chooser', 'library']);
      expect(hub.registry.openRecordId, 'apple-record');
      expect(find.byType(ContentDialog), findsNothing);
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets(
      'a failure the user must act on for a library that is not open shows nothing',
      (tester) async {
        final hub = await _hub(open: 'local-record');
        final contextOf = await _context(tester);
        final automatic = automaticFor(hub, contextOf, _Host([_refused]));

        automatic.needsSignIn('apple-record');
        await tester.pumpAndSettle();

        expect(events, isEmpty);
        expect(hub.registry.openRecordId, 'local-record');
        expect(find.byType(ContentDialog), findsNothing);
      },
    );

    testWidgets(
      'a failure that may pass keeps the library open, shows nothing, and is tried again '
      'when the network is back',
      (tester) async {
        final hub = await _hub(open: 'apple-record')
          ..signInFailure = _hubRefused(
            RegisterLibraryFailureKind.remoteUnreachable,
          );
        final contextOf = await _context(tester);
        final host = _Host([_authorization]);
        final automatic = automaticFor(hub, contextOf, host);

        automatic.needsSignIn('apple-record');
        await tester.pumpAndSettle();
        expect(hub.signIns, hasLength(1));
        expect(events, isEmpty);
        expect(hub.registry.openRecordId, 'apple-record');
        expect(find.byType(ContentDialog), findsNothing);

        // Requests that fail soon after are left alone, so nothing loops.
        now = now.add(const Duration(seconds: 30));
        automatic.needsSignIn('apple-record');
        automatic.accessLost(AppleMusicHost.family);
        await tester.pumpAndSettle();
        expect(hub.signIns, hasLength(1));

        // The network coming back does not wait for the cooldown.
        hub.signInFailure = null;
        automatic.networkBack();
        await tester.pumpAndSettle();
        expect(hub.signIns, hasLength(2));
        expect(hub.signedIn, isTrue);
        expect(policy.awaitingRetry, isEmpty);
        expect(events, isEmpty);
        expect(find.byType(ContentDialog), findsNothing);

        // A network that comes back again finds nothing left to sign in.
        automatic.networkBack();
        await tester.pumpAndSettle();
        expect(hub.signIns, hasLength(2));
      },
    );

    testWidgets(
      'a host that gives no token is tried again after the cooldown',
      (tester) async {
        final hub = await _hub(open: 'apple-record');
        final contextOf = await _context(tester);
        final host = _Host([_noToken, _authorization]);
        final automatic = automaticFor(hub, contextOf, host);

        automatic.needsSignIn('apple-record');
        await tester.pumpAndSettle();
        now = now.add(const Duration(seconds: 59));
        automatic.needsSignIn('apple-record');
        await tester.pumpAndSettle();
        expect(host.calls, [false]);
        expect(find.byType(ContentDialog), findsNothing);

        now = now.add(const Duration(seconds: 2));
        automatic.needsSignIn('apple-record');
        await tester.pumpAndSettle();
        expect(host.calls, [false, false]);
        expect(hub.signIns, hasLength(1));
        expect(events, isEmpty);
        expect(find.byType(ContentDialog), findsNothing);
      },
    );

    testWidgets(
      'a library that needs a sign-in right after one worked waits out the cooldown',
      (tester) async {
        final hub = await _hub(open: 'apple-record');
        final contextOf = await _context(tester);
        final host = _Host([_authorization]);
        final automatic = automaticFor(hub, contextOf, host);

        automatic.needsSignIn('apple-record');
        await tester.pumpAndSettle();
        now = now.add(const Duration(seconds: 30));
        automatic.needsSignIn('apple-record');
        await tester.pumpAndSettle();
        expect(host.calls, [false]);

        now = now.add(const Duration(seconds: 31));
        automatic.needsSignIn('apple-record');
        await tester.pumpAndSettle();
        expect(host.calls, [false, false]);
        expect(hub.signIns, hasLength(2));
        expect(find.byType(ContentDialog), findsNothing);
      },
    );

    testWidgets(
      'the dialog stays up for Esc and a click beside it while it signs in',
      (tester) async {
        final hub = await _hub();
        final contextOf = await _context(tester);
        final host = _Host([_refused, _refused, _authorization]);
        final results = <bool>[];
        unawaited(
          signInOnDeviceAgain(
            contextOf(),
            hub.signInsWith(host, policy: policy),
            'apple-record',
          ).then(results.add),
        );
        await tester.pumpAndSettle();
        expect(find.text(_strings.loginFailed), findsOneWidget);

        final gate = Completer<void>();
        hub.signInGate = gate.future;
        await tester.tap(find.text(_strings.playOnRetry));
        await tester.pump();
        await tester.sendKeyEvent(LogicalKeyboardKey.escape);
        await tester.pump();
        await tester.tapAt(const Offset(4, 4));
        await tester.pump();
        expect(find.text(_strings.loginFailed), findsOneWidget);
        expect(results, isEmpty);

        gate.complete();
        await tester.pumpAndSettle();
        expect(results, [true]);
        expect(find.byType(ContentDialog), findsNothing);
      },
    );

    testWidgets('Esc closes the dialog while it waits for the user', (
      tester,
    ) async {
      final hub = await _hub();
      final contextOf = await _context(tester);
      final results = <bool>[];
      unawaited(
        signInOnDeviceAgain(
          contextOf(),
          hub.signInsWith(_Host([_refused]), policy: policy),
          'apple-record',
        ).then(results.add),
      );
      await tester.pumpAndSettle();
      expect(find.text(_strings.loginFailed), findsOneWidget);

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(results, [false]);
      expect(find.byType(ContentDialog), findsNothing);
    });

    testWidgets('Esc closes the dialog after a Retry that failed', (
      tester,
    ) async {
      final hub = await _hub();
      final contextOf = await _context(tester);
      final host = _Host([_refused]);
      final results = <bool>[];
      unawaited(
        signInOnDeviceAgain(
          contextOf(),
          hub.signInsWith(host, policy: policy),
          'apple-record',
        ).then(results.add),
      );
      await tester.pumpAndSettle();
      final asked = host.calls.length;
      await tester.tap(find.text(_strings.playOnRetry));
      await tester.pumpAndSettle();
      expect(host.calls.length, greaterThan(asked));
      expect(find.text(_strings.loginFailed), findsOneWidget);

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(results, [false]);
      expect(find.byType(ContentDialog), findsNothing);
    });

    testWidgets('a click beside the dialog closes it like Esc', (tester) async {
      final hub = await _hub();
      final contextOf = await _context(tester);
      final results = <bool>[];
      unawaited(
        signInOnDeviceAgain(
          contextOf(),
          hub.signInsWith(_Host([_refused]), policy: policy),
          'apple-record',
        ).then(results.add),
      );
      await tester.pumpAndSettle();
      expect(find.text(_strings.loginFailed), findsOneWidget);

      await tester.tapAt(const Offset(4, 4));
      await tester.pumpAndSettle();
      expect(results, [false]);
      expect(find.byType(ContentDialog), findsNothing);
    });

    testWidgets('an open during a flow for lost access joins it', (
      tester,
    ) async {
      final hub = await _hub(open: 'apple-record');
      final host = _Host([_refused, _refused, _authorization]);
      final signIns = hub.signInsWith(host, policy: policy);
      final page = GlobalKey();
      await tester.pumpWidget(
        Provider<DeviceSignIns>.value(
          value: signIns,
          child: _app(SizedBox(key: page)),
        ),
      );
      final automatic = AutomaticDeviceSignIn(
        signIns: signIns,
        dialogContext: () => page.currentContext!,
        leaveLibrary: () async => hub.registry.onClosed(),
        showLibrary: () {},
      );
      automatic.accessLost(AppleMusicHost.family);
      await tester.pumpAndSettle();
      expect(find.text(_strings.loginFailed), findsOneWidget);

      var opened = false;
      unawaited(
        openLibraryWithSignIn(
          page.currentContext!,
          hub.registry,
          'apple-record',
        ).then((value) => opened = value),
      );
      await tester.pumpAndSettle();
      expect(find.text(_strings.loginFailed), findsOneWidget);
      await tester.tap(find.text(_strings.playOnRetry));
      await tester.pumpAndSettle();

      expect(opened, isTrue);
      expect(host.calls, [false, true, false]);
      expect(hub.signIns, hasLength(1));
      expect(hub.registry.openRecordId, 'apple-record');
      expect(find.byType(ContentDialog), findsNothing);
    });

    group('failures that may pass by themselves, many times in a row', () {
      late _Hub hub;
      late AutomaticDeviceSignIn automatic;

      Future<void> setUpFlow(WidgetTester tester, HostedPlayerHost host) async {
        hub = await _hub(open: 'apple-record');
        final contextOf = await _context(tester);
        automatic = automaticFor(hub, contextOf, host);
      }

      /// Lets a sign-in fail [times] times, each after the cooldown of the one
      /// before.
      Future<void> fail(WidgetTester tester, int times) async {
        for (var index = 0; index < times; index++) {
          now = now.add(signInCooldown * 2);
          automatic.accessLost(AppleMusicHost.family);
          await tester.pumpAndSettle();
        }
      }

      testWidgets(
        'three in a row are told like a failure the user must act on',
        (tester) async {
          await setUpFlow(tester, _Host([_noToken]));

          await fail(tester, 2);
          expect(events, isEmpty);
          expect(hub.registry.openRecordId, 'apple-record');
          expect(find.byType(ContentDialog), findsNothing);

          await fail(tester, 1);
          expect(events, ['chooser']);
          expect(hub.registry.openRecordId, isNull);
          expect(find.text(_strings.loginFailed), findsOneWidget);
          expect(find.text(_strings.appleMusicNoToken), findsOneWidget);
          expect(find.text(_strings.playOnRetry), findsOneWidget);
          expect(find.text(_strings.close), findsOneWidget);
        },
      );

      testWidgets('a success in between starts the count again', (
        tester,
      ) async {
        final host = _Host([
          _noToken,
          _noToken,
          _authorization,
          _noToken,
          _noToken,
        ]);
        await setUpFlow(tester, host);

        await fail(tester, 5);

        expect(host.calls, hasLength(5));
        expect(events, isEmpty);
        expect(hub.registry.openRecordId, 'apple-record');
        expect(find.byType(ContentDialog), findsNothing);
      });

      testWidgets('after the dialog it takes three more to show it again', (
        tester,
      ) async {
        await setUpFlow(tester, _Host([_noToken]));
        await fail(tester, 3);
        expect(find.text(_strings.appleMusicNoToken), findsOneWidget);

        await tester.tap(find.text(_strings.close));
        await tester.pumpAndSettle();
        expect(find.byType(ContentDialog), findsNothing);
        await hub.registry.open('apple-record');

        await fail(tester, 2);
        expect(events, ['chooser']);
        expect(find.byType(ContentDialog), findsNothing);
        expect(hub.registry.openRecordId, 'apple-record');

        await fail(tester, 1);
        expect(events, ['chooser', 'chooser']);
        expect(find.text(_strings.appleMusicNoToken), findsOneWidget);
      });

      for (final (name, failure) in [
        (
          'a service that cannot be reached',
          _hubRefused(RegisterLibraryFailureKind.remoteUnreachable),
        ),
        ('a hub that does not answer', TimeoutException('late')),
      ]) {
        testWidgets('$name never escalates', (tester) async {
          await setUpFlow(tester, _Host([_authorization]));
          hub.signInFailure = failure;

          await fail(tester, 6);

          expect(hub.signIns, hasLength(6));
          expect(events, isEmpty);
          expect(hub.registry.openRecordId, 'apple-record');
          expect(find.byType(ContentDialog), findsNothing);
        });
      }

      testWidgets('a host that reports no network never escalates', (
        tester,
      ) async {
        final host = _OfflineHost([_noToken]);
        await setUpFlow(tester, host);

        await fail(tester, 6);

        expect(host.calls, hasLength(6));
        expect(events, isEmpty);
        expect(hub.registry.openRecordId, 'apple-record');
        expect(find.byType(ContentDialog), findsNothing);
      });
    });
  });

  group('library settings', () {
    Future<void> pumpSettings(
      WidgetTester tester,
      _Hub hub,
      HostedPlayerHost? host,
    ) async {
      final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
      final responsive = ResponsiveProvider(sizes, () async => null);
      final broadcast = _Broadcast();
      final manager = _LibraryManager();
      final state = LibraryOpenStateProvider(null, hub.registry);
      addTearDown(sizes.dispose);
      addTearDown(responsive.dispose);
      addTearDown(broadcast.dispose);
      addTearDown(manager.dispose);
      addTearDown(state.dispose);
      await tester.pumpWidget(
        MultiProvider(
          providers: [
            ChangeNotifierProvider<LibraryRegistryProvider>.value(
              value: hub.registry,
            ),
            ChangeNotifierProvider<LibraryOpenStateProvider>.value(
              value: state,
            ),
            Provider<DeviceSignIns>.value(value: hub.signInsWith(host)),
            ChangeNotifierProvider<BroadcastProvider>.value(value: broadcast),
            ChangeNotifierProvider<LibraryManagerProvider>.value(
              value: manager,
            ),
            ChangeNotifierProvider<ScreenSizeProvider>.value(value: sizes),
            ChangeNotifierProvider<ResponsiveProvider>.value(value: responsive),
          ],
          child: _app(
            Navigator(
              key: runeWithNavigationBarAndPlaybackControllorNavigatorKey,
              onGenerateRoute: (settings) => FluentPageRoute(
                settings: settings,
                builder: (_) => settings.name == '/library'
                    ? const Text('Library page')
                    : SettingsLibraryPage(
                        loadLibraries: () async =>
                            const ListLocalLibrariesResponse(libraries: []),
                      ),
              ),
            ),
          ),
        ),
      );
      await tester.pump();
    }

    Finder appleRow() => find.ancestor(
      of: find.text(_appleRecord.alias),
      matching: find.byType(ListTile),
    );

    List<String?> rowButtons(WidgetTester tester) => tester
        .widgetList<Button>(
          find.descendant(of: appleRow(), matching: find.byType(Button)),
        )
        .map((button) => (button.child as Text).data)
        .toList();

    for (final (state, record, open) in [
      ('open', _signedInAppleRecord, 'apple-record'),
      ('signed in', _signedInAppleRecord, null),
      ('not signed in', _appleRecord, null),
    ]) {
      testWidgets(
        'the row of a library that is $state has Switch To and Remove',
        (tester) async {
          final hub = await _hub(records: [record], open: open);
          await pumpSettings(tester, hub, _Host([_authorization]));

          expect(find.text(_appleRecord.alias), findsOneWidget);
          expect(find.text(_strings.appleMusicSubtitle), findsOneWidget);
          await tester.tap(find.text(_appleRecord.alias));
          await tester.pump();
          expect(rowButtons(tester), [
            _strings.switchTo,
            _strings.removeLibrary,
          ]);
          await tester.pump(const Duration(seconds: 1));
        },
        variant: TargetPlatformVariant.only(TargetPlatform.macOS),
      );
    }

    testWidgets('Switch To signs the library in and opens it', (tester) async {
      // Read by the router when Switch To moves to the library pages.
      cafeMode = false;
      final host = _Host([_authorization]);
      final hub = await _hub();
      await pumpSettings(tester, hub, host);

      await tester.tap(find.text(_appleRecord.alias));
      await tester.pump();
      await tester.tap(
        find.descendant(of: appleRow(), matching: find.text(_strings.switchTo)),
      );
      await tester.pumpAndSettle();

      expect(host.calls, [false]);
      expect(hub.registry.openRecordId, 'apple-record');
      expect(find.text('Library page'), findsOneWidget);
      await tester.pump(const Duration(seconds: 1));
    });

    testWidgets(
      'Switch To that does not sign in tells why and stays here without an error',
      (tester) async {
        final hub = await _hub(open: 'local-record');
        await pumpSettings(tester, hub, _Host([_refused]));

        await tester.tap(find.text(_appleRecord.alias));
        await tester.pump();
        await tester.tap(
          find.descendant(
            of: appleRow(),
            matching: find.text(_strings.switchTo),
          ),
        );
        await tester.pumpAndSettle();
        expect(find.text(_strings.appleMusicAccessDenied), findsOneWidget);
        await tester.tap(find.widgetWithText(Button, _strings.close));
        await tester.pumpAndSettle();

        expect(find.byType(ContentDialog), findsNothing);
        expect(find.text(_strings.unableToOpenLibrary), findsNothing);
        expect(hub.registry.openRecordId, 'local-record');
        expect(find.text(_appleRecord.alias), findsOneWidget);
        await tester.pump(const Duration(seconds: 1));
      },
      variant: TargetPlatformVariant.only(TargetPlatform.macOS),
    );

    testWidgets('Remove removes the record', (tester) async {
      final hub = await _hub();
      await pumpSettings(tester, hub, null);

      await tester.tap(find.text(_appleRecord.alias));
      await tester.pump();
      await tester.tap(
        find.descendant(
          of: appleRow(),
          matching: find.text(_strings.removeLibrary),
        ),
      );
      await tester.pump();

      expect(hub.removed, ['apple-record']);
      expect(hub.registry.records.map((record) => record.recordId), [
        'local-record',
      ]);
      await tester.pump(const Duration(seconds: 1));
    });
  });
}

class _Broadcast extends ChangeNotifier implements BroadcastProvider {
  @override
  List<ClientSummary> get users => const [];

  @override
  void fetchUsers() {}

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _LibraryManager extends ChangeNotifier implements LibraryManagerProvider {
  @override
  Stream<RequestFailure> get failureFeedback => const Stream.empty();

  @override
  dynamic noSuchMethod(Invocation invocation) => null;
}
