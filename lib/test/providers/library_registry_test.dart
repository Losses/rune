import 'dart:async';
import 'dart:typed_data';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rinf/rinf.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/library_registry.dart';
import 'package:rune/utils/api/request_failure.dart';

LibraryRecordSummary _record() => const LibraryRecordSummary(
  recordId: 'record-1',
  alias: 'Music',
  kind: LibraryRecordKindSummaryLocal(libraryId: 'library-1', path: '/music'),
);

ListLibraryRecordsResponse _listed({OpenRecord? open}) =>
    ListLibraryRecordsResponse(
      records: [_record()],
      open: open,
      lastOpenRecord: open?.recordId,
    );

OpenLibraryResponse _opened(String recordId, int callId) => OpenLibraryResponse(
  callId: Uint64.fromBigInt(BigInt.from(callId)),
  recordId: recordId,
  generation: Uint64.fromBigInt(BigInt.from(9)),
  result: const OpenLibraryResultOpened(),
);

void main() {
  test(
    'loads record/open identity and opens the requested record id',
    () async {
      final openResponses =
          StreamController<RustSignalPack<OpenLibraryResponse>>.broadcast();
      final requests = <OpenLibraryRequest>[];
      final provider = LibraryRegistryProvider(
        loadRecords: () async => _listed(
          open: OpenRecord(
            recordId: 'record-1',
            generation: Uint64.fromBigInt(BigInt.from(8)),
          ),
        ),
        openRecord: (recordId, callId) async {
          final request = OpenLibraryRequest(
            callId: Uint64.fromBigInt(BigInt.from(callId)),
            recordId: recordId,
          );
          final response = await awaitResponse(
            'OpenLibraryRequest',
            openResponses.stream,
            callId: callId,
            responseCallId: (signal) => signal.message.callId.toInt(),
            send: () {
              requests.add(request);
              scheduleMicrotask(
                () => openResponses.add(
                  RustSignalPack(_opened(recordId, callId), Uint8List(0)),
                ),
              );
            },
          );
          return response.message;
        },
      );
      addTearDown(provider.dispose);
      addTearDown(openResponses.close);

      await provider.ready;
      expect(provider.records.single.recordId, 'record-1');
      expect(provider.openRecordId, 'record-1');
      expect(provider.openGeneration, 8);

      await provider.open('record-1');
      expect(requests.map((request) => request.recordId), ['record-1']);
    },
  );

  test(
    'removing the open record closes it before removing its record',
    () async {
      final sequence = <String>[];
      final provider = LibraryRegistryProvider(
        loadRecords: () async => _listed(
          open: OpenRecord(
            recordId: 'record-1',
            generation: Uint64.fromBigInt(BigInt.from(8)),
          ),
        ),
        closeRecord: () async {
          sequence.add('close');
        },
        removeRecord: (recordId) async {
          expect(recordId, 'record-1');
          sequence.add('remove');
          return const RemoveLibraryRecordResponse(
            outcome: RemoveLibraryRecordOutcomeRemoved(),
          );
        },
      );
      addTearDown(provider.dispose);

      await provider.ready;
      await provider.remove('record-1');
      expect(sequence, ['close', 'remove']);
      expect(provider.records, isEmpty);
    },
  );

  testWidgets('folder registration asks for a mode, retries, then opens', (
    tester,
  ) async {
    final modes = <LibraryInitializeMode?>[];
    final opened = <String>[];
    final provider = LibraryRegistryProvider(
      loadRecords: () async =>
          ListLibraryRecordsResponse(records: const [], lastOpenRecord: null),
      registerLocal: (path, alias, mode) async {
        expect(path, '/new/music');
        expect(alias, 'Music');
        modes.add(mode);
        if (mode == null) {
          return const RegisterLibraryResponse(
            outcome: RegisterLibraryOutcomeNeedsInitializeMode(),
          );
        }
        return RegisterLibraryResponse(
          record: _record(),
          outcome: const RegisterLibraryOutcomeRegistered(),
        );
      },
      openRecord: (recordId, callId) async {
        opened.add(recordId);
        return _opened(recordId, callId);
      },
    );
    addTearDown(provider.dispose);
    await provider.ready;

    await tester.pumpWidget(
      FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Builder(
          builder: (context) => FilledButton(
            onPressed: () => registerAndOpenLocalFolder(
              context,
              provider,
              '/new/music',
              'Music',
            ),
            child: const Text('Add folder'),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Add folder'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Local hard drive'));
    await tester.pumpAndSettle();

    expect(modes, [null, LibraryInitializeMode.portable]);
    expect(opened, ['record-1']);
  });
}
