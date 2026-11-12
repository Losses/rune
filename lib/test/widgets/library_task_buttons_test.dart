import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/library_manager.dart';
import 'package:rune/providers/library_open_state.dart';
import 'package:rune/providers/library_registry.dart';
import 'package:rune/utils/api/request_failure.dart';
import 'package:rune/widgets/library_task_button.dart';

class _LibraryManager extends ChangeNotifier implements LibraryManagerProvider {
  final StreamController<RequestFailure> _failures =
      StreamController<RequestFailure>.broadcast();

  @override
  Stream<RequestFailure> get failureFeedback => _failures.stream;

  @override
  dynamic noSuchMethod(Invocation invocation) => null;

  @override
  void dispose() {
    _failures.close();
    super.dispose();
  }
}

Future<(LibraryOpenStateProvider, LibraryRegistryProvider)> _openState(
  LibraryRecordSummary record,
) async {
  final registry = LibraryRegistryProvider(
    loadRecords: () async => ListLibraryRecordsResponse(
      records: [record],
      invalidRecords: const [],
      open: OpenRecord(
        recordId: record.recordId,
        generation: Uint64.fromBigInt(BigInt.one),
      ),
      lastOpenRecord: null,
    ),
  );
  final state = LibraryOpenStateProvider(null, registry);
  await state.initialOpen;
  return (state, registry);
}

Future<void> _expectTaskButtons(
  WidgetTester tester,
  LibraryRecordSummary record, {
  required bool visible,
}) async {
  final (openState, registry) = await _openState(record);
  final manager = _LibraryManager();
  addTearDown(openState.dispose);
  addTearDown(registry.dispose);
  addTearDown(manager.dispose);

  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider<LibraryOpenStateProvider>.value(
          value: openState,
        ),
        ChangeNotifierProvider<LibraryManagerProvider>.value(value: manager),
      ],
      child: const FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Column(
          children: [
            ScanLibraryButton(title: 'Scan'),
            AnalyzeLibraryButton(title: 'Analyze'),
          ],
        ),
      ),
    ),
  );
  await tester.pump();
  expect(find.text('Scan'), visible ? findsOneWidget : findsNothing);
  expect(find.text('Analyze'), visible ? findsOneWidget : findsNothing);
  await tester.pumpWidget(const SizedBox());
}

void main() {
  testWidgets('catalog task buttons appear for replicas, not remote control', (
    tester,
  ) async {
    await _expectTaskButtons(
      tester,
      const LibraryRecordSummary(
        recordId: 'replica-record',
        alias: 'Replica',
        kind: LibraryRecordKindSummaryReplica(
          libraryId: 'replica-library',
          ownerFingerprint: 'owner',
        ),
        syncAccess: LibraryAccessSummary.read,
      ),
      visible: true,
    );
    await _expectTaskButtons(
      tester,
      const LibraryRecordSummary(
        recordId: 'remote-record',
        alias: 'Remote control',
        kind: LibraryRecordKindSummaryRemoteControl(fingerprint: 'renderer'),
      ),
      visible: false,
    );
  });
}
