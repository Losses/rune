import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/library_manager.dart';
import 'package:rune/providers/library_open_state.dart';
import 'package:rune/widgets/library_task_button.dart';

class _LibraryPath extends ChangeNotifier implements LibraryOpenStateProvider {
  @override
  LibraryRecordSummary? get openRecord => const LibraryRecordSummary(
    recordId: 'library',
    alias: 'Library',
    kind: LibraryRecordKindSummaryLocal(libraryId: 'id', path: '/library'),
  );

  @override
  bool get hasCatalog => true;

  @override
  String? get openLocalFolderPath => '/library';

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

RequestFailedResponse _scanFailure(Uint64 callId) => RequestFailedResponse(
  request: 'different diagnostic name',
  callId: callId,
  kind: RequestErrorKind.conflict,
  detail: 'Scan was refused',
  cause: const RequestFailureCauseLibraryTaskAlreadyRunning(
    task: LibraryTaskKind.scan,
  ),
);

void main() {
  testWidgets('scan start failure is shown by its task button call id', (
    tester,
  ) async {
    final failures = StreamController<RequestFailedResponse>.broadcast();
    final requests = <ScanAudioLibraryRequest>[];
    final manager = LibraryManagerProvider(
      requestFailures: failures.stream,
      sendScanRequest: requests.add,
    );
    addTearDown(() async {
      manager.dispose();
      await failures.close();
    });

    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider<LibraryOpenStateProvider>(
            create: (_) => _LibraryPath(),
          ),
          ChangeNotifierProvider<LibraryManagerProvider>.value(value: manager),
        ],
        child: FluentApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: ScaffoldPage(
            content: LibraryTaskButton(
              title: 'Scan',
              progressTitle: 'Scanning',
              progress: null,
              onPressedStart: (manager, recordId, callId) =>
                  manager.scanLibrary(recordId, callId: callId),
              onPressedCancel: (_, _) {},
              isTaskWorking: (_, _, _) => false,
            ),
          ),
        ),
      ),
    );

    await tester.tap(find.text('Scan'));
    await tester.pumpAndSettle();

    expect(requests, hasLength(1));
    expect(requests.single.callId, isNotNull);
    failures.add(_scanFailure(Uint64.fromBigInt(BigInt.zero)));
    await tester.pump();
    expect(
      find.text('Scanning: This is not available right now.'),
      findsNothing,
    );

    final matchingFailureProcessed = manager.failureFeedback.first;
    failures.add(_scanFailure(requests.single.callId!));
    await matchingFailureProcessed;
    await tester.pumpAndSettle();
    expect(
      find.text('Scanning: This is not available right now.'),
      findsOneWidget,
    );
  });
}
