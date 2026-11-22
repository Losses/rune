import 'dart:typed_data';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../l10n/app_localizations_en.dart';
import '../../main.dart';
import '../../providers/library_open_state.dart';
import '../../providers/library_registry.dart';
import '../../providers/license.dart';
import '../../providers/status.dart';
import '../../utils/api/request_failure.dart';

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
