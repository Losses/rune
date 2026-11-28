import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/main.dart' as app;
import 'package:rune/providers/library_manager.dart';
import 'package:rune/providers/library_registry.dart';
import 'package:rune/utils/settings_manager.dart';
import 'package:rune/utils/api/set_sync_settings.dart';
import 'package:saf_util/saf_util.dart';

import 'e2e_library_flow_test.dart' as lifecycle;

// On a disposable emulator with sqlite3 and all-files access, provide the
// fresh folder's document ID with --dart-define=RUNE_SAF_TREE_ID=... .
// Run once per --dart-define=RUNE_SAF_LOOKUP_FAULT value (provider, database).
void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  const fault = String.fromEnvironment('RUNE_SAF_LOOKUP_FAULT');
  testWidgets('a $fault lookup failure preserves the scanned track', (
    tester,
  ) async {
    const treeId = String.fromEnvironment('RUNE_SAF_TREE_ID');
    expect(['database', 'provider'], contains(fault));
    final parts = treeId.split(':');
    final volume = parts.first == 'primary'
        ? '/sdcard'
        : '/storage/${parts.first}';
    final root = Directory('$volume/${parts.last}');
    final track = File('${root.path}/startup_0.ogg');
    expect(await root.list().map((entry) => entry.path).toList(), [track.path]);
    await Directory('${root.path}/still-mounted').create();
    if (fault == 'database') await track.copy('${root.path}/cached.ogg');

    app.main(const []);
    final appRoot = find.byType(FluentApp);
    await lifecycle.waitFor(
      tester,
      () => appRoot.evaluate().isNotEmpty,
      description: 'Rune to start',
    );
    final context = tester.element(appRoot);
    final registry = context.read<LibraryRegistryProvider>();
    final manager = context.read<LibraryManagerProvider>();
    await registry.ready;
    final tree = (await SafUtil().pickDirectory(
      initialUri: Uri(
        scheme: 'content',
        host: 'com.android.externalstorage.documents',
        pathSegments: ['document', treeId],
      ).toString(),
      persistablePermission: true,
      writePermission: true,
    ))!;
    expect(Uri.parse(tree.uri).pathSegments, [
      'tree',
      treeId,
      'document',
      treeId,
    ]);
    final record = (await registry.registerLocalFolder(
      tree.uri,
      tree.name,
      LibraryInitializeMode.redirected,
    )).record!;
    await registry.open(record.recordId);
    await setSyncSettings(
      const SyncSettingsChange(
        missingHostFile: MissingHostFilePolicy.removeEverywhere,
      ),
    );
    Future<void> scan() async {
      await manager.scanLibrary(record.recordId);
      expect(
        await manager.waitForScanToComplete(record.recordId),
        TaskStatus.finished,
      );
    }

    Future<String> query(String database, String sql) async {
      final result = await Process.run('/system/bin/sqlite3', [database, sql]);
      expect(result.exitCode, 0, reason: result.stderr.toString());
      return result.stdout.toString().trim();
    }

    Future<String> cache(String sql) =>
        query('${root.path}/.rune/.android-fs.db', sql);
    await scan();
    final storageKey = (await File(
      '${root.path}/.rune/.redirect',
    ).readAsString()).trim();
    final catalog = '${await getSettingsPath()}/$storageKey/.0.db';
    expect(await File(catalog).exists(), isTrue);
    Future<String> tracks() => query(
      catalog,
      'SELECT file_name,source_uuid,local_available FROM media_files ORDER BY file_name',
    );
    final before = await tracks();
    expect(before, contains(RegExp(r'startup_0\.ogg\|[^|]+\|1')));
    if (fault == 'database') expect(before, contains('cached.ogg|'));
    // The live sync candidates identify the versions an observed removal would retire.
    Future<String> versions() => query(
      catalog,
      "SELECT hex(object),hex(digest) FROM sync_candidates WHERE CAST(collection AS TEXT)='tracks' "
      "AND live_until IS NULL ORDER BY object,digest",
    );
    final published = await versions();
    expect(published, isNotEmpty);
    expect(
      await cache(
        "DELETE FROM fs_cache WHERE path='startup_0.ogg'; SELECT changes()",
      ),
      '1',
    );
    if (fault == 'provider') {
      // The open cache can still list the root while its tree URI stops working.
      final parked = await root.rename('${root.path}.offline');
      try {
        await scan();
      } finally {
        await parked.rename(root.path);
      }
    } else {
      // Reject the refresh and the subsequent live lookup's write-through.
      await cache(
        "CREATE TRIGGER fail_cache_insert BEFORE INSERT ON fs_cache "
        "WHEN NEW.path='startup_0.ogg' BEGIN SELECT RAISE(ABORT,'test cache write failure'); END",
      );
      try {
        await scan();
      } finally {
        await cache('DROP TRIGGER fail_cache_insert');
      }
    }
    expect(
      await cache("SELECT count(*) FROM fs_cache WHERE path='startup_0.ogg'"),
      '0',
    );
    expect(
      await tracks(),
      before,
      reason: '$fault lookup failure removed a track',
    );
    expect(await versions(), published);
    await scan();
    expect(await tracks(), before);
    expect(
      await cache("SELECT count(*) FROM fs_cache WHERE path='startup_0.ogg'"),
      '1',
    );

    final held = await track.rename('${root.path}/original.held');
    try {
      await scan();
      expect(await tracks(), isNot(contains('startup_0.ogg|')));
    } finally {
      await held.rename(track.path);
    }
  }, timeout: const Timeout(Duration(minutes: 4)));
}
