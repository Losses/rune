import 'dart:async';
import 'dart:typed_data';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../l10n/app_localizations_en.dart';
import '../../main.dart';
import '../../providers/crash.dart';
import '../../providers/library_open_state.dart';
import '../../providers/library_registry.dart';
import '../../providers/license.dart';
import '../../providers/linux_custom_window_controls.dart';
import '../../providers/responsive_providers.dart';
import '../../providers/router_path.dart';
import '../../providers/status.dart';
import '../../providers/visualizer_lab.dart';
import '../../providers/visualizer_scenes.dart';
import '../../screens/bsod/bsod.dart';
import '../../utils/api/request_failure.dart';
import '../../utils/device_sign_in.dart';
import '../../utils/show_visualizer_import.dart';
import '../../widgets/ax_reveal/widgets/reveal_effect_context.dart';
import '../../widgets/router/rune_router_frame_implementation.dart';

final _s = AppLocalizationsEn();

void main() {
  testWidgets(
    'only the current replica shows one retry notice until recovery',
    (tester) async {
      await _pumpApp(
        tester,
        record: const LibraryRecordSummary(
          recordId: 'replica-record',
          alias: 'Shared Music',
          kind: LibraryRecordKindSummaryReplica(
            libraryId: 'catalog',
            ownerFingerprint: 'owner',
          ),
          shareSync: false,
          syncAccess: LibraryAccessSummary.read,
        ),
      );
      await tester.pumpAndSettle();

      Future<void> report(String owner, bool retrying) async {
        _receive(
          'PeerMetadataSyncStatusUpdated',
          PeerMetadataSyncStatusUpdated(
            libraryId: 'catalog',
            ownerFingerprint: owner,
            retrying: retrying,
          ).bincodeSerialize(),
        );
        await _frames(tester);
      }

      final warning = find.text(_s.peerMetadataSyncRetrying);
      await report('other owner', true);
      expect(warning, findsNothing);
      await report('owner', true);
      expect(warning, findsOneWidget);
      await report('owner', true);
      expect(warning, findsOneWidget);
      await report('owner', false);
      expect(warning, findsNothing);
      await report('owner', true);
      expect(warning, findsOneWidget);
      await _close(tester);
      await report('owner', true);
      expect(warning, findsNothing);
    },
  );

  for (final (kind, message) in [
    (RequestErrorKind.notFound, _s.requestItemMissing),
    (RequestErrorKind.invalidInput, _s.requestInvalid),
  ]) {
    testWidgets('a $kind failure dialog shows its localized message', (
      tester,
    ) async {
      await _pumpPage(tester);
      final context = tester.element(find.byKey(_anchor));
      unawaited(
        showRequestFailureDialog(
          context,
          RequestFailure('SetLikedRequest', kind, 'refused'),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.byType(ContentDialog), findsOneWidget);
      expect(find.text(_s.unknownError), findsOneWidget);
      expect(find.text(message), findsOneWidget);

      await _close(tester);
      expect(find.byType(ContentDialog), findsNothing);
    });
  }

  for (final (result, title, text) in [
    (
      const VisualizerImported('Night Drive'),
      _s.visualizerScenes,
      _s.visualizerSceneImported('Night Drive'),
    ),
    (
      const VisualizerImportRejected('Night Drive', 'not a scene'),
      _s.visualizerSceneImportFailed('Night Drive'),
      'not a scene',
    ),
    (
      const VisualizerImportUnanswered('Night Drive'),
      _s.visualizerSceneImportFailed('Night Drive'),
      _s.visualizerNoAnswer,
    ),
  ]) {
    testWidgets('a ${result.runtimeType} scene import shows in a dialog', (
      tester,
    ) async {
      await _pumpPage(tester);
      final context = tester.element(find.byKey(_anchor));
      unawaited(showVisualizerImport(context, result));
      await tester.pumpAndSettle();
      expect(find.byType(ContentDialog), findsOneWidget);
      expect(find.text(title), findsOneWidget);
      expect(find.text(text), findsOneWidget);

      await _close(tester);
      expect(find.byType(ContentDialog), findsNothing);
    });
  }

  testWidgets(
    'a failure no caller waits for shows in a dialog over the pages',
    (tester) async {
      await _pumpApp(tester);
      _receive(
        'RequestFailedResponse',
        const RequestFailedResponse(
          request: 'SetVisualizerLogRequest',
          kind: RequestErrorKind.unavailable,
          detail: 'not running',
        ).bincodeSerialize(),
      );
      await _showNotice(tester);
      expect(find.byType(ContentDialog), findsOneWidget);
      expect(find.text(_s.requestUnavailable), findsOneWidget);
      expect(tester.takeException(), isNull);

      await _close(tester);
      expect(find.byType(ContentDialog), findsNothing);
    },
  );

  testWidgets('a failure its caller waits for is left to the caller', (
    tester,
  ) async {
    await _pumpApp(tester);
    _receive(
      'RequestFailedResponse',
      RequestFailedResponse(
        request: 'SetLikedRequest',
        callId: Uint64.fromBigInt(BigInt.one),
        kind: RequestErrorKind.unavailable,
        detail: 'not running',
      ).bincodeSerialize(),
    );
    await _showNotice(tester);
    expect(find.byType(ContentDialog), findsNothing);
  });

  testWidgets('an internal failure no caller waits for shows in a dialog', (
    tester,
  ) async {
    await _pumpApp(tester);
    _receive(
      'RequestFailedResponse',
      const RequestFailedResponse(
        request: 'FetchPlaylistOccurrencesRequest',
        kind: RequestErrorKind.internal,
        detail: 'no such table: media_file_playlists',
      ).bincodeSerialize(),
    );
    await _showNotice(tester);
    expect(find.byType(ContentDialog), findsOneWidget);
    // The title, and the message of a failure with nothing more to say.
    expect(find.text(_s.unknownError), findsNWidgets(2));
    expect(tester.takeException(), isNull);
  });

  testWidgets(
    'router frame keeps the current page after a typed internal failure and still shows Bsod on a fatal crash',
    (tester) async {
      disableBrandingAnimation = true;
      cafeMode = false;
      $router.update('/playlists/detail', null);
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(800, 320);
      addTearDown(tester.view.reset);

      final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
      final responsive = ResponsiveProvider(sizes, () async => null);
      final crash = CrashProvider();
      final library = _OpenLibrary();
      final status = _Status();
      final scenes = _Scenes();
      final lab = _Lab();
      final windowControls = _WindowControls();
      addTearDown(sizes.dispose);
      addTearDown(responsive.dispose);
      addTearDown(crash.dispose);
      addTearDown(library.dispose);
      addTearDown(status.dispose);
      addTearDown(scenes.dispose);
      addTearDown(lab.dispose);
      addTearDown(windowControls.dispose);

      await tester.pumpWidget(
        MultiProvider(
          providers: [
            ChangeNotifierProvider.value(value: sizes),
            ChangeNotifierProvider.value(value: responsive),
            ChangeNotifierProvider.value(value: crash),
            ChangeNotifierProvider.value(value: $router),
            ChangeNotifierProvider<LibraryOpenStateProvider>.value(
              value: library,
            ),
            ChangeNotifierProvider<PlaybackStatusProvider>.value(value: status),
            ChangeNotifierProvider<VisualizerScenesProvider>.value(
              value: scenes,
            ),
            ChangeNotifierProvider<VisualizerLabProvider>.value(value: lab),
            ChangeNotifierProvider<LinuxCustomWindowControlsProvider>.value(
              value: windowControls,
            ),
          ],
          child: const FluentApp(
            localizationsDelegates: AppLocalizations.localizationsDelegates,
            supportedLocales: AppLocalizations.supportedLocales,
            home: RevealEffectContext(
              child: RuneRouterFrameImplementation(
                child: Center(child: Text('Playlist page')),
              ),
            ),
          ),
        ),
      );
      await tester.pump();
      expect(find.text('Playlist page'), findsOneWidget);
      expect(find.byType(Bsod), findsNothing);

      // A recoverable database error emits only RequestFailedResponse.
      _receive(
        'RequestFailedResponse',
        RequestFailedResponse(
          request: 'FetchPlaylistOccurrencesRequest',
          callId: Uint64.fromBigInt(BigInt.from(2)),
          kind: RequestErrorKind.internal,
          detail: 'no such table: media_file_playlists',
        ).bincodeSerialize(),
      );
      await tester.pump();
      await tester.pump();
      expect(crash.report, isNull);
      expect($router.path, '/playlists/detail');
      expect(find.text('Playlist page'), findsOneWidget);
      expect(find.byType(Bsod), findsNothing);

      // A fatal crash still replaces the frame with Bsod.
      _receive(
        'CrashResponse',
        const CrashResponse(detail: 'fatal player failure').bincodeSerialize(),
      );
      await tester.pump();
      await tester.pump();
      expect(crash.report, 'fatal player failure');
      expect(find.byType(Bsod), findsOneWidget);
      expect(find.text('fatal player failure'), findsOneWidget);
      expect(find.text('Playlist page'), findsNothing);

      await tester.pumpWidget(const SizedBox());
      await tester.pump(const Duration(seconds: 2));
    },
  );

  testWidgets('notices show one at a time, and a repeated one only once', (
    tester,
  ) async {
    await _pumpApp(tester);
    void fail(RequestErrorKind kind) => _receive(
      'RequestFailedResponse',
      RequestFailedResponse(
        request: 'SetLikedRequest',
        kind: kind,
        detail: 'refused',
      ).bincodeSerialize(),
    );
    fail(RequestErrorKind.unavailable);
    fail(RequestErrorKind.unavailable);
    fail(RequestErrorKind.notFound);
    await _showNotice(tester);
    expect(find.byType(ContentDialog), findsOneWidget);
    expect(find.text(_s.requestUnavailable), findsOneWidget);

    await _close(tester);
    expect(find.byType(ContentDialog), findsOneWidget);
    expect(find.text(_s.requestItemMissing), findsOneWidget);

    await _close(tester);
    expect(find.byType(ContentDialog), findsNothing);

    // A failure that was closed shows again when it happens again.
    fail(RequestErrorKind.unavailable);
    await _showNotice(tester);
    expect(find.text(_s.requestUnavailable), findsOneWidget);
  });

  testWidgets('the reconnecting dialog closes once the library is back', (
    tester,
  ) async {
    await _pumpApp(tester);
    // The dialog's progress ring never settles, so frames are pumped instead.
    final reconnecting = _s.remoteLibraryReconnecting('Bedroom');
    void disconnect() => _receive(
      'RemoteLibraryReconnecting',
      const RemoteLibraryReconnecting(
        fingerprint: 'b',
        alias: 'Bedroom',
      ).bincodeSerialize(),
    );
    void reconnect() => _receive(
      'RemoteLibraryReconnected',
      const RemoteLibraryReconnected(
        fingerprint: 'b',
        alias: 'Bedroom',
      ).bincodeSerialize(),
    );

    disconnect();
    await _frames(tester);
    expect(find.byType(ContentDialog), findsOneWidget);
    expect(find.text(reconnecting), findsOneWidget);
    reconnect();
    await _frames(tester);
    expect(find.byType(ContentDialog), findsNothing);

    // A library that is back before the dialog's first frame closes it too.
    disconnect();
    reconnect();
    await _frames(tester);
    expect(find.byType(ContentDialog), findsNothing);

    // It also leaves from under a dialog opened over it.
    disconnect();
    await _frames(tester);
    _receive(
      'RendererLinkLost',
      const RendererLinkLost(
        fingerprint: 'fingerprint',
        alias: 'Kitchen',
      ).bincodeSerialize(),
    );
    await _frames(tester);
    expect(find.byType(ContentDialog), findsNWidgets(2));
    reconnect();
    await _frames(tester);
    expect(find.text(reconnecting), findsNothing);
    expect(find.text(_s.rendererLinkLost('Kitchen')), findsOneWidget);

    // Closed by the user, it stays closed while the library reconnects.
    await tester.tap(find.text(_s.close).last);
    await _frames(tester);
    disconnect();
    await _frames(tester);
    await tester.tap(find.text(_s.close).last);
    await _frames(tester);
    expect(find.byType(ContentDialog), findsNothing);
  });

  for (final (signal, bytes, text) in [
    (
      'LibrarySyncStatusUpdated',
      const LibrarySyncStatusUpdated(
        libraryId: 'library-uuid',
        shareFailure: ShareSyncFailure.probeFailed,
      ).bincodeSerialize(),
      _s.libraryStorageFailure,
    ),
    (
      'RendererLinkLost',
      const RendererLinkLost(
        fingerprint: 'fingerprint',
        alias: 'Bedroom',
      ).bincodeSerialize(),
      _s.rendererLinkLost('Bedroom'),
    ),
  ]) {
    testWidgets('a $signal notice shows in a dialog over the pages', (
      tester,
    ) async {
      await _pumpApp(tester);
      _receive(signal, bytes);
      await _showNotice(tester);
      expect(find.byType(ContentDialog), findsOneWidget);
      expect(find.text(text), findsOneWidget);
      expect(tester.takeException(), isNull);

      await _close(tester);
      expect(find.byType(ContentDialog), findsNothing);
    });
  }
}

/// Pumps [RuneLifecycle] above the root navigator, as the app does, with the
/// providers it reads.
Future<void> _pumpApp(
  WidgetTester tester, {
  LibraryRecordSummary? record,
}) async {
  // main() sets these before the app starts: cafe mode from the settings, and
  // whether the last library is reopened.
  cafeMode = false;
  hasStartupLibrary = false;
  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider(
          create: (_) => LibraryRegistryProvider(
            loadRecords: () async => ListLibraryRecordsResponse(
              records: [if (record != null) record],
              invalidRecords: const [],
              open: record == null
                  ? null
                  : OpenRecord(
                      recordId: record.recordId,
                      generation: Uint64.fromBigInt(BigInt.one),
                    ),
            ),
          ),
        ),
        Provider(
          create: (context) => DeviceSignIns(
            context.read<LibraryRegistryProvider>(),
            methods: const {},
          ),
        ),
        ChangeNotifierProvider(
          create: (context) => LibraryOpenStateProvider(
            null,
            context.read<LibraryRegistryProvider>(),
          ),
        ),
        ChangeNotifierProvider(
          create: (_) =>
              PlaybackStatusProvider(statusStream: const Stream.empty()),
        ),
        ChangeNotifierProvider<LicenseProvider>(create: (_) => _License()),
      ],
      child: FluentApp(
        navigatorKey: rootNavigatorKey,
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        builder: (context, child) => RuneLifecycle(child!),
        home: const SizedBox.shrink(),
      ),
    ),
  );
}

/// Delivers a signal the way Rust does, through the generated dispatch.
void _receive(String signal, Uint8List bytes) =>
    assignRustSignal[signal]!(bytes, Uint8List(0));

/// The signal arrives in a microtask, and its dialog opens on a later frame.
Future<void> _showNotice(WidgetTester tester) async {
  await tester.pump();
  await tester.pump();
  await tester.pumpAndSettle();
}

/// Lets a signal arrive and a dialog open or close, without settling.
Future<void> _frames(WidgetTester tester) async {
  await tester.pump();
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 500));
  await tester.pump(const Duration(milliseconds: 500));
}

const _anchor = Key('anchor');

/// A page to open a dialog from, found by [_anchor].
Future<void> _pumpPage(WidgetTester tester) => tester.pumpWidget(
  FluentApp(
    localizationsDelegates: AppLocalizations.localizationsDelegates,
    supportedLocales: AppLocalizations.supportedLocales,
    home: const SizedBox.shrink(key: _anchor),
  ),
);

/// Presses the Close button of the dialog on top.
Future<void> _close(WidgetTester tester) async {
  await tester.tap(find.text(_s.close).last);
  await tester.pumpAndSettle();
}

/// Off the desktop, [RuneLifecycle] only listens to the license.
class _License extends ChangeNotifier implements LicenseProvider {
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _OpenLibrary extends ChangeNotifier implements LibraryOpenStateProvider {
  @override
  bool get hasOpenLibrary => true;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _Status extends ChangeNotifier implements PlaybackStatusProvider {
  @override
  final playbackStatus = PlaybackStatusState()..state = PlaybackState.paused;

  @override
  bool get notReady => false;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _Scenes extends ChangeNotifier implements VisualizerScenesProvider {
  @override
  Stream<VisualizerImport> get imports => const Stream.empty();

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _Lab extends ChangeNotifier implements VisualizerLabProvider {
  @override
  bool get overlayOpen => false;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _WindowControls extends ChangeNotifier
    implements LinuxCustomWindowControlsProvider {
  @override
  bool get enabled => false;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
