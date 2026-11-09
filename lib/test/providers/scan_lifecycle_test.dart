import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/library_open_state.dart';
import 'package:rune/screens/welcome/scanning.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/providers/library_manager.dart';

class FakeLibraryPath extends ChangeNotifier implements LibraryOpenStateProvider {
  @override
  LibraryRecordSummary? get openRecord => const LibraryRecordSummary(
    recordId: 'test-record',
    alias: 'Test',
    kind: LibraryRecordKindSummaryLocal(libraryId: 'test-library', path: '/test/library'),
  );
  @override
  String? get openLocalFolderPath => '/test/library';
  @override
  bool get hasCatalog => true;
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test(
    'scan terminal events settle waiters and ignore replaced tasks',
    () async {
      final requests = <ScanAudioLibraryRequest>[];
      final manager = LibraryManagerProvider(sendScanRequest: requests.add);
      addTearDown(manager.dispose);
      const recordId = 'test-record';
      await manager.scanLibrary(recordId);
      final old = requests.last.scanId;
      final firstWait = manager.waitForScanToComplete(recordId);
      await manager.scanLibrary(recordId);
      expect(await firstWait, TaskStatus.cancelled);
      final active = requests.last.scanId;
      final wait = manager.waitForScanToComplete(recordId);

      void terminal(String id, String status) {
        final response = ScanAudioLibraryResponse(
          scanId: id,
          status: status,
          error: status == 'failed' ? 'injected failure' : '',
          recordId: recordId,
          progress: 0,
        );
        assignRustSignal['ScanAudioLibraryResponse']!(
          response.bincodeSerialize(),
          Uint8List(0),
        );
      }

      terminal(old, 'success');
      await Future<void>.delayed(Duration.zero);
      expect(manager.getScanTaskProgress(recordId)?.status, TaskStatus.working);
      terminal(active, 'failed');
      expect(await wait, TaskStatus.failed);
      expect(await manager.waitForScanToComplete(recordId), TaskStatus.failed);
      expect(manager.getScanTaskProgress(recordId)?.error, 'injected failure');

      await manager.scanLibrary(recordId);
      final cancelled = manager.waitForScanToComplete(recordId);
      terminal(requests.last.scanId, 'cancelled');
      expect(await cancelled, TaskStatus.cancelled);
      expect(await manager.waitForScanToComplete(recordId), TaskStatus.cancelled);

      await manager.scanLibrary(recordId);
      final cleared = manager.waitForScanToComplete(recordId);
      final stale = requests.last.scanId;
      manager.clearAll(recordId);
      expect(await cleared, TaskStatus.cancelled);
      terminal(stale, 'success');
      await Future<void>.delayed(Duration.zero);
      expect(manager.getScanTaskProgress(recordId), isNull);
    },
  );
  test('successful initialization navigates only for success', () async {
    final requests = <ScanAudioLibraryRequest>[];
    final routes = <String>[];
    final manager = LibraryManagerProvider(
      sendScanRequest: requests.add,
      navigate: routes.add,
    );
    addTearDown(manager.dispose);
    await manager.scanLibrary('test-record', isInitializeTask: true);
    final response = ScanAudioLibraryResponse(
      scanId: requests.last.scanId,
      status: 'success',
      error: '',
      recordId: 'test-record',
      progress: 1,
    );
    assignRustSignal['ScanAudioLibraryResponse']!(
      response.bincodeSerialize(),
      Uint8List(0),
    );
    await Future<void>.delayed(Duration.zero);
    expect(routes, ['/scanning', '/library']);
    for (final status in ['failed', 'cancelled']) {
      await manager.scanLibrary('test-record', isInitializeTask: true);
      final response = ScanAudioLibraryResponse(
        scanId: requests.last.scanId,
        status: status,
        error: '',
        recordId: 'test-record',
        progress: 0,
      );
      assignRustSignal['ScanAudioLibraryResponse']!(
        response.bincodeSerialize(),
        Uint8List(0),
      );
      await Future<void>.delayed(Duration.zero);
    }
    expect(routes, ['/scanning', '/library', '/scanning', '/scanning']);
  });

  testWidgets('failed initialization screen offers recovery', (tester) async {
    final requests = <ScanAudioLibraryRequest>[];
    final manager = LibraryManagerProvider(
      sendScanRequest: requests.add,
      navigate: (_) {},
    );
    addTearDown(manager.dispose);
    await manager.scanLibrary('test-record', isInitializeTask: true);
    final response = ScanAudioLibraryResponse(
      scanId: requests.last.scanId,
      status: 'failed',
      error: 'injected failure',
      recordId: 'test-record',
      progress: 0,
    );
    assignRustSignal['ScanAudioLibraryResponse']!(
      response.bincodeSerialize(),
      Uint8List(0),
    );
    await tester.pump();
    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider<LibraryOpenStateProvider>(
            create: (_) => FakeLibraryPath(),
          ),
          ChangeNotifierProvider<LibraryManagerProvider>.value(value: manager),
        ],
        child: const FluentApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: ScanningPage(),
        ),
      ),
    );
    await tester.pump();
    expect(find.textContaining('injected failure'), findsOneWidget);
    expect(find.byType(Button), findsNWidgets(2));
    await manager.scanLibrary('test-record', isInitializeTask: true);
    final cancelled = ScanAudioLibraryResponse(
      scanId: requests.last.scanId,
      status: 'cancelled',
      error: '',
      recordId: 'test-record',
      progress: 0,
    );
    assignRustSignal['ScanAudioLibraryResponse']!(
      cancelled.bincodeSerialize(),
      Uint8List(0),
    );
    await tester.pump();
    expect(find.byType(Button), findsNWidgets(2));
    await tester.pump(const Duration(milliseconds: 100));
    await tester.pumpWidget(const SizedBox());
    await tester.pump(const Duration(milliseconds: 100));
  });
}
