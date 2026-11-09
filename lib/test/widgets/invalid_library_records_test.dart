import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/broadcast.dart';
import 'package:rune/providers/library_manager.dart';
import 'package:rune/providers/library_open_state.dart';
import 'package:rune/providers/library_registry.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/screens/home/home.dart';
import 'package:rune/screens/home/widgets/welcome.dart';
import 'package:rune/screens/settings_library/settings_library.dart';

const _invalidRecord = InvalidLibraryRecordSummary(
  recordId: 'invalid-1',
  alias: 'Broken library',
  kind: LibraryRecordKindSummaryLocal(libraryId: 'library-1', path: '/music'),
  reason: InvalidRecordReasonSummary.nativeInvalid,
);

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
  dynamic noSuchMethod(Invocation invocation) => null;
}

Future<(ScreenSizeProvider, ResponsiveProvider)> _responsive() async {
  final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
  final responsive = ResponsiveProvider(sizes, () async => null);
  return (sizes, responsive);
}

void main() {
  testWidgets(
    'invalid last-open record falls back to the usable welcome page',
    (tester) async {
      final registry = LibraryRegistryProvider(
        loadRecords: () async => const ListLibraryRecordsResponse(
          records: [],
          invalidRecords: [_invalidRecord],
          lastOpenRecord: 'invalid-1',
        ),
        openRecord: (recordId, callId) async => OpenLibraryResponse(
          callId: Uint64.fromBigInt(BigInt.from(callId)),
          recordId: recordId,
          generation: Uint64.fromBigInt(BigInt.zero),
          result: const OpenLibraryResultFailed(
            value: OpenLibraryFailure(
              kind: OpenLibraryFailureKind.invalidRecord,
              detail: 'Native record fields are invalid',
            ),
          ),
        ),
      );
      final state = LibraryOpenStateProvider(null, registry);
      final (sizes, responsive) = await _responsive();
      addTearDown(state.dispose);
      addTearDown(registry.dispose);
      addTearDown(responsive.dispose);
      addTearDown(sizes.dispose);

      expect(shouldOpenLibraryAtStartup(null, registry), isFalse);
      expect(await state.initialOpen, isFalse);

      await tester.pumpWidget(
        MultiProvider(
          providers: [
            ChangeNotifierProvider<LibraryRegistryProvider>.value(
              value: registry,
            ),
            ChangeNotifierProvider<ScreenSizeProvider>.value(value: sizes),
            ChangeNotifierProvider<ResponsiveProvider>.value(value: responsive),
          ],
          child: const FluentApp(
            localizationsDelegates: AppLocalizations.localizationsDelegates,
            supportedLocales: AppLocalizations.supportedLocales,
            home: HomePage(),
          ),
        ),
      );
      expect(find.byType(WelcomePage), findsOneWidget);
      await tester.pump(const Duration(seconds: 1));
    },
  );

  testWidgets('invalid library row has only Remove and removes the record', (
    tester,
  ) async {
    final removed = <String>[];
    final registry = LibraryRegistryProvider(
      loadRecords: () async => const ListLibraryRecordsResponse(
        records: [],
        invalidRecords: [_invalidRecord],
        lastOpenRecord: null,
      ),
      removeRecord: (recordId) async {
        removed.add(recordId);
        return const RemoveLibraryRecordResponse(
          outcome: RemoveLibraryRecordOutcomeRemoved(),
        );
      },
    );
    final broadcast = _Broadcast();
    final manager = _LibraryManager();
    final (sizes, responsive) = await _responsive();
    addTearDown(registry.dispose);
    addTearDown(broadcast.dispose);
    addTearDown(manager.dispose);
    addTearDown(responsive.dispose);
    addTearDown(sizes.dispose);
    await registry.ready;

    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider<LibraryRegistryProvider>.value(
            value: registry,
          ),
          ChangeNotifierProvider<BroadcastProvider>.value(value: broadcast),
          ChangeNotifierProvider<LibraryManagerProvider>.value(value: manager),
          ChangeNotifierProvider<ScreenSizeProvider>.value(value: sizes),
          ChangeNotifierProvider<ResponsiveProvider>.value(value: responsive),
        ],
        child: FluentApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: SettingsLibraryPage(
            loadLibraries: () async =>
                const ListLocalLibrariesResponse(libraries: []),
          ),
        ),
      ),
    );
    await tester.pump();
    expect(find.text('Broken library'), findsOneWidget);
    expect(find.text("This library's settings can't be read."), findsOneWidget);

    final row = find.ancestor(
      of: find.text('Broken library'),
      matching: find.byType(ListTile),
    );
    await tester.tap(find.text('Broken library'));
    await tester.pump();
    expect(
      find.descendant(of: row, matching: find.byType(Button)),
      findsOneWidget,
    );
    expect(
      find.descendant(of: row, matching: find.text('Remove')),
      findsOneWidget,
    );
    await tester.tap(find.descendant(of: row, matching: find.text('Remove')));
    await tester.pump();

    expect(removed, ['invalid-1']);
    expect(registry.invalidRecords, isEmpty);
    await tester.pump(const Duration(seconds: 1));
  });
}
