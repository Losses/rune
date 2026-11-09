import 'package:flutter_test/flutter_test.dart';
import '../bindings/bindings.dart';
import '../l10n/app_localizations_en.dart';
import '../providers/library_open_state.dart';
import '../providers/library_registry.dart';
import '../utils/library_ui/library_ui_state.dart';

class _OpenStateFixture {
  _OpenStateFixture(this.state, this.registry);

  final LibraryOpenStateProvider state;
  final LibraryRegistryProvider registry;

  void dispose() {
    state.dispose();
    registry.dispose();
  }
}

Future<_OpenStateFixture> _stateFor(LibraryRecordSummary record) async {
  final registry = LibraryRegistryProvider(
    loadRecords: () async => ListLibraryRecordsResponse(
      records: [record],
      open: OpenRecord(
        recordId: record.recordId,
        generation: Uint64.fromBigInt(BigInt.one),
      ),
      lastOpenRecord: null,
    ),
  );
  final state = LibraryOpenStateProvider(null, registry);
  await state.initialOpen;
  return _OpenStateFixture(state, registry);
}

void main() {
  test('local folder labels use the typed folder path', () {
    expect(localFolderName('/Users/me/Music'), 'Music');
    expect(
      filesAppLocation('/Users/me/Music', onThisDevice: 'Device', iCloudDrive: 'iCloud'),
      '/Users/me/Music',
    );
    expect(describeLibraryPath(AppLocalizationsEn(), '/Users/me/Music'), '/Users/me/Music');
  });

  test('open state derives mode and folder access from the typed record', () async {
    final localFixture = await _stateFor(
      const LibraryRecordSummary(
        recordId: 'local-record',
        alias: 'Music',
        kind: LibraryRecordKindSummaryLocal(
          libraryId: 'local-library',
          path: '/music',
        ),
      ),
    );
    addTearDown(localFixture.dispose);
    final local = localFixture.state;
    expect(local.isRemoteControl, isFalse);
    expect(local.isReadOnlyReplica, isFalse);
    expect(local.openLocalFolderPath, '/music');

    final replicaFixture = await _stateFor(
      const LibraryRecordSummary(
        recordId: 'replica-record',
        alias: 'Shared Music',
        kind: LibraryRecordKindSummaryReplica(
          libraryId: 'replica-library',
          ownerFingerprint: 'peer',
        ),
        syncAccess: LibraryAccessSummary.read,
      ),
    );
    addTearDown(replicaFixture.dispose);
    final replica = replicaFixture.state;
    expect(replica.isReadOnlyReplica, isTrue);
    expect(replica.openLocalFolderPath, isNull);

    final remoteFixture = await _stateFor(
      const LibraryRecordSummary(
        recordId: 'remote-record',
        alias: 'Renderer',
        kind: LibraryRecordKindSummaryRemoteControl(
          fingerprint: 'renderer',
        ),
      ),
    );
    addTearDown(remoteFixture.dispose);
    final remote = remoteFixture.state;
    expect(remote.isRemoteControl, isTrue);
    expect(remote.isReadOnlyReplica, isFalse);
    expect(remote.openLocalFolderPath, isNull);
  });
}
