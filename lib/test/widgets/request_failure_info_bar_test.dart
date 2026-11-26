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
import '../../widgets/ax_reveal/widgets/reveal_effect_context.dart';
import '../../widgets/router/rune_router_frame_implementation.dart';

void main() {
  for (final (kind, message) in [
    (RequestErrorKind.notFound, AppLocalizationsEn().requestItemMissing),
    (RequestErrorKind.invalidInput, AppLocalizationsEn().requestInvalid),
  ]) {
    testWidgets('a $kind failure notice shows its localized message', (
      tester,
    ) async {
      const anchor = Key('anchor');
      await tester.pumpWidget(
        FluentApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: const SizedBox.shrink(key: anchor),
        ),
      );

      showRequestFailureInfoBar(
        tester.element(find.byKey(anchor)),
        RequestFailure('SetLikedRequest', kind, 'refused'),
      );
      // The notice is built on the next frame and fades in after a timer,
      // which settling alone does not advance.
      await tester.pump();
      await tester.pump(const Duration(seconds: 1));
      await tester.pumpAndSettle();
      expect(find.text(message), findsOneWidget);

      // Let the notice close on its own before the tree is torn down.
      await tester.pump(const Duration(seconds: 4));
      await tester.pumpAndSettle();
      expect(find.text(message), findsNothing);
    });
  }

  testWidgets('a failure no caller waits for shows its notice over the pages', (
    tester,
  ) async {
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
    final message = AppLocalizationsEn().requestUnavailable;
    expect(find.text(message), findsOneWidget);
    expect(tester.takeException(), isNull);

    await tester.tap(find.byIcon(FluentIcons.chrome_close));
    await tester.pumpAndSettle();
    expect(find.text(message), findsNothing);
    // The timer that would have closed the notice still runs out.
    await tester.pump(const Duration(seconds: 4));
  });

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
    expect(find.byType(InfoBar), findsNothing);
  });

  testWidgets('an internal failure no caller waits for shows its notice', (
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
    final message = AppLocalizationsEn().unknownError;
    expect(find.text(message), findsOneWidget);
    expect(tester.takeException(), isNull);

    await tester.pump(const Duration(seconds: 4));
    await tester.pumpAndSettle();
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

  testWidgets('a notice outlasts the page it was shown over', (tester) async {
    await _pumpApp(tester);
    final s = AppLocalizationsEn();
    _receive(
      'RequestFailedResponse',
      const RequestFailedResponse(
        request: 'SetVisualizerLogRequest',
        kind: RequestErrorKind.unavailable,
        detail: 'not running',
      ).bincodeSerialize(),
    );
    await _showNotice(tester);
    rootNavigatorKey.currentState!.pushReplacement(
      FluentPageRoute<void>(builder: (_) => const SizedBox.shrink()),
    );
    await tester.pumpAndSettle();
    // The next notice rebuilds the overlay with the first one still shown.
    _receive(
      'RequestFailedResponse',
      const RequestFailedResponse(
        request: 'SetLikedRequest',
        kind: RequestErrorKind.notFound,
        detail: 'gone',
      ).bincodeSerialize(),
    );
    await _showNotice(tester);
    expect(tester.takeException(), isNull);
    expect(find.text(s.requestUnavailable), findsOneWidget);
    expect(find.text(s.requestItemMissing), findsOneWidget);

    await tester.pump(const Duration(seconds: 4));
    await tester.pumpAndSettle();
  });

  for (final (signal, bytes, text) in [
    (
      'LibrarySyncStatusUpdated',
      const LibrarySyncStatusUpdated(
        libraryId: 'library-uuid',
        shareFailure: ShareSyncFailure.probeFailed,
      ).bincodeSerialize(),
      AppLocalizationsEn().libraryStorageFailure,
    ),
    (
      'RendererLinkLost',
      const RendererLinkLost(
        fingerprint: 'fingerprint',
        alias: 'Bedroom',
      ).bincodeSerialize(),
      AppLocalizationsEn().rendererLinkLost('Bedroom'),
    ),
  ]) {
    testWidgets('a $signal notice shows over the pages', (tester) async {
      await _pumpApp(tester);
      _receive(signal, bytes);
      await _showNotice(tester);
      expect(find.text(text), findsOneWidget);
      expect(tester.takeException(), isNull);

      await tester.pump(const Duration(seconds: 4));
      await tester.pumpAndSettle();
    });
  }
}

/// Pumps [RuneLifecycle] above the root navigator, as the app does, with the
/// providers it reads.
Future<void> _pumpApp(WidgetTester tester) async {
  // main() sets these before the app starts: cafe mode from the settings, and
  // whether the last library is reopened.
  cafeMode = false;
  hasStartupLibrary = false;
  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider(
          create: (_) => LibraryRegistryProvider(
            loadRecords: () async => const ListLibraryRecordsResponse(
              records: [],
              invalidRecords: [],
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

/// The signal arrives in a microtask, the notice is built on the next frame,
/// and it fades in after a timer.
Future<void> _showNotice(WidgetTester tester) async {
  await tester.pump();
  await tester.pump();
  await tester.pump(const Duration(seconds: 1));
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
