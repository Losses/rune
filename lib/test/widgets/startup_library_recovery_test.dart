import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../config/routes.dart';
import '../../l10n/app_localizations.dart';
import '../../l10n/app_localizations_en.dart';
import '../../main.dart';
import '../../providers/library_home.dart';
import '../../providers/library_open_state.dart';
import '../../providers/library_registry.dart';
import '../../providers/license.dart';
import '../../providers/request_capabilities.dart';
import '../../providers/responsive_providers.dart';
import '../../providers/status.dart';
import '../../screens/home/widgets/select_library.dart';
import '../../screens/library_home/library_home.dart';
import '../../utils/api/request_failure.dart';
import '../../widgets/ax_reveal/widgets/reveal_effect_context.dart';

const _missingRecord = LibraryRecordSummary(
  recordId: 'missing-record',
  alias: 'Missing',
  kind: LibraryRecordKindSummaryLocal(
    libraryId: 'missing-library',
    path: '/music/missing',
  ),
);

const _remoteRecord = LibraryRecordSummary(
  recordId: 'remote-record',
  alias: 'Remote',
  kind: LibraryRecordKindSummarySubsonic(
    endpoint: 'https://music.example',
    account: 'listener',
  ),
);

const _availableRecord = LibraryRecordSummary(
  recordId: 'available-record',
  alias: 'Available',
  kind: LibraryRecordKindSummaryLocal(
    libraryId: 'available-library',
    path: '/music/available',
  ),
);

const _invalidRecord = InvalidLibraryRecordSummary(
  recordId: 'invalid-record',
  alias: 'Broken',
  kind: LibraryRecordKindSummaryLocal(
    libraryId: 'broken-library',
    path: '/music/broken',
  ),
  reason: InvalidRecordReasonSummary.nativeInvalid,
);

RequestFailure _openFailure(String recordId) => switch (recordId) {
  'missing-record' => const RequestFailure(
    'OpenLibraryRequest',
    RequestErrorKind.unavailable,
    'The library folder is missing',
    RequestFailureCauseLibraryOpenFailed(
      kind: OpenLibraryFailureKind.mediaFolderMissing,
    ),
  ),
  'remote-record' => const RequestFailure(
    'OpenLibraryRequest',
    RequestErrorKind.unavailable,
    'The device could not be reached',
    RequestFailureCauseLibraryOpenFailed(
      kind: OpenLibraryFailureKind.remoteUnreachable,
    ),
  ),
  'invalid-record' => const RequestFailure(
    'OpenLibraryRequest',
    RequestErrorKind.invalidInput,
    'Native record fields are invalid',
    RequestFailureCauseLibraryOpenFailed(
      kind: OpenLibraryFailureKind.invalidRecord,
    ),
  ),
  _ => throw StateError('Unexpected open of $recordId'),
};

OpenLibraryResponse _opened(String recordId, int callId) => OpenLibraryResponse(
  callId: Uint64.fromBigInt(BigInt.from(callId)),
  recordId: recordId,
  generation: Uint64.fromBigInt(BigInt.one),
);

/// Off the desktop, [RuneLifecycle] only listens to the license.
class _License extends ChangeNotifier implements LicenseProvider {
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

/// A library that keeps no summaries, so the home does not ask Rust for them.
class _NoSummaryCapabilities extends RequestCapabilitiesProvider {
  _NoSummaryCapabilities() : super(changes: const Stream.empty());

  @override
  RequestCapability get complexQueryRequest => const RequestCapabilityHidden();
}

Future<void> _pumpStartup(
  WidgetTester tester, {
  required List<LibraryRecordSummary> records,
  List<InvalidLibraryRecordSummary> invalidRecords = const [],
  required String lastOpenRecord,
  required Future<OpenLibraryResponse> Function(String recordId, int callId)
  openRecord,
}) async {
  // main() sets these before the app starts.
  cafeMode = false;
  final registry = LibraryRegistryProvider(
    loadRecords: () async => ListLibraryRecordsResponse(
      records: records,
      invalidRecords: invalidRecords,
      lastOpenRecord: lastOpenRecord,
    ),
    openRecord: openRecord,
  );
  addTearDown(registry.dispose);
  await registry.ready;
  hasStartupLibrary = shouldOpenLibraryAtStartup(null, registry);

  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider<LibraryRegistryProvider>.value(value: registry),
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
        ChangeNotifierProvider(
          create: (_) => LibraryHomeProvider(readStored: () async => null),
        ),
        ChangeNotifierProvider(
          create: (_) => ScreenSizeProvider.withWriter(null, (_) async {}),
        ),
        ChangeNotifierProvider(
          create: (context) => ResponsiveProvider(
            context.read<ScreenSizeProvider>(),
            () async => null,
          ),
        ),
        ChangeNotifierProvider<RequestCapabilitiesProvider>(
          create: (_) => _NoSummaryCapabilities(),
        ),
      ],
      child: FluentApp(
        navigatorKey: rootNavigatorKey,
        locale: const Locale('en'),
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        initialRoute: hasStartupLibrary ? '/library' : '/',
        onGenerateRoute: (settings) {
          final builder = routes[settings.name];
          return FluentPageRoute<void>(
            settings: settings,
            builder: (context) =>
                builder?.call(context) ?? const SizedBox.shrink(),
          );
        },
        builder: (context, child) =>
            RuneLifecycle(RevealEffectContext(child: child!)),
      ),
    ),
  );
}

/// Lets the startup open finish, the failed route leave and the notice show.
Future<void> _settleStartup(WidgetTester tester) async {
  await tester.pump();
  await tester.pump();
  await tester.pump();
  await tester.pumpAndSettle();
}

Future<void> _expectChooserAfterFailure(
  WidgetTester tester, {
  required String message,
}) async {
  await _settleStartup(tester);
  final s = AppLocalizationsEn();
  expect(find.byType(SelectLibraryPage), findsOneWidget);
  expect(find.text(s.unableToOpenLibrary), findsOneWidget);
  expect(
    tester.widget<SelectableText>(find.byType(SelectableText)).data,
    message,
  );

  await tester.tap(find.widgetWithText(FilledButton, s.close));
  await tester.pumpAndSettle();
  expect(find.text(s.unableToOpenLibrary), findsNothing);

  // Lets the lifecycle's tray refresh timer run out before teardown.
  await tester.pump(const Duration(seconds: 1));
}

void main() {
  testWidgets(
    'a missing last library falls back to the chooser and opens another',
    (tester) async {
      await _pumpStartup(
        tester,
        records: const [_missingRecord, _availableRecord],
        lastOpenRecord: _missingRecord.recordId,
        openRecord: (recordId, callId) async {
          if (recordId == _availableRecord.recordId) {
            return _opened(recordId, callId);
          }
          throw _openFailure(recordId);
        },
      );

      await _expectChooserAfterFailure(
        tester,
        message: AppLocalizationsEn().libraryMediaFolderMissing,
      );

      await tester.tap(find.text('available'));
      await tester.pumpAndSettle();
      expect(find.byType(LibraryHomePage), findsOneWidget);
      expect(find.byType(SelectLibraryPage), findsNothing);
      final registry = Provider.of<LibraryRegistryProvider>(
        tester.element(find.byType(LibraryHomePage)),
        listen: false,
      );
      expect(registry.openRecordId, _availableRecord.recordId);

      await tester.pump(const Duration(seconds: 1));
    },
  );

  testWidgets('an unreachable remote last library falls back to the chooser', (
    tester,
  ) async {
    await _pumpStartup(
      tester,
      records: const [_remoteRecord, _availableRecord],
      lastOpenRecord: _remoteRecord.recordId,
      openRecord: (recordId, callId) async => throw _openFailure(recordId),
    );

    await _expectChooserAfterFailure(
      tester,
      message: AppLocalizationsEn().libraryRemoteUnreachable,
    );
  });

  testWidgets('a removed invalid last record starts at the chooser', (
    tester,
  ) async {
    await _pumpStartup(
      tester,
      records: const [_availableRecord],
      invalidRecords: const [_invalidRecord],
      lastOpenRecord: _invalidRecord.recordId,
      openRecord: (recordId, callId) async => throw _openFailure(recordId),
    );

    await _settleStartup(tester);
    expect(find.byType(SelectLibraryPage), findsOneWidget);
    expect(find.text(AppLocalizationsEn().unableToOpenLibrary), findsNothing);

    await tester.pump(const Duration(seconds: 1));
  });

  testWidgets('a library that restores still opens without the chooser', (
    tester,
  ) async {
    await _pumpStartup(
      tester,
      records: const [_availableRecord],
      lastOpenRecord: _availableRecord.recordId,
      openRecord: (recordId, callId) async => _opened(recordId, callId),
    );

    await _settleStartup(tester);
    expect(find.byType(LibraryHomePage), findsOneWidget);
    expect(find.byType(SelectLibraryPage), findsNothing);
    expect(find.text(AppLocalizationsEn().unableToOpenLibrary), findsNothing);

    await tester.pump(const Duration(seconds: 1));
  });
}
