import 'dart:convert';
import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/main.dart' as app;
import 'package:rune/providers/library_manager.dart';
import 'package:rune/providers/library_registry.dart';
import 'package:rune/utils/api/close_library.dart';
import 'package:rune/utils/api/fetch_media_files.dart';
import 'package:saf_util/saf_util.dart';

import 'e2e_library_flow_test.dart' as lifecycle;

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('scan and reopen a portable Android library', (tester) async {
    const treeId = String.fromEnvironment('RUNE_SAF_TREE_ID');
    expect(Platform.isAndroid, isTrue);
    expect(treeId, isNotEmpty);
    app.main(const []);
    final appRoot = find.byType(FluentApp);
    await lifecycle.waitFor(
      tester,
      () => appRoot.evaluate().isNotEmpty,
      description: 'the real app to finish startup',
    );
    final registry = tester.element(appRoot).read<LibraryRegistryProvider>();
    final manager = tester.element(appRoot).read<LibraryManagerProvider>();
    await registry.ready;

    final saf = SafUtil();
    final tree = (await saf.pickDirectory(
      initialUri: Uri(
        scheme: 'content',
        host: 'com.android.externalstorage.documents',
        pathSegments: ['document', treeId],
      ).toString(),
      persistablePermission: true,
      writePermission: true,
    ))!;
    final root = Uri.parse(tree.uri);
    expect(root.pathSegments, ['tree', treeId, 'document', treeId]);
    String indexDocument(String name) => root
        .replace(
          pathSegments: ['tree', treeId, 'document', '$treeId/.rune/$name'],
        )
        .toString();
    expect(
      await saf.hasPersistedPermission(tree.uri, checkWrite: true),
      isTrue,
    );
    final index = indexDocument('.0.db');
    expect(await saf.stat(index, false), isNull);

    final response = await registry.registerLocalFolder(
      tree.uri,
      tree.name,
      LibraryInitializeMode.portable,
    );
    final record = response.record!;
    await registry.open(record.recordId);
    await manager.scanLibrary(record.recordId);
    expect(
      await manager.waitForScanToComplete(record.recordId),
      TaskStatus.finished,
    );
    final track = (await fetchMediaFiles(0, 2)).single;
    expect(track.title, 'startup_0.ogg');
    expect(track.track, isNotEmpty);
    await closeLibrary(tester.element(appRoot));

    // Read the index through the document provider after closing the library.
    final fd = await saf.getFileDescriptor(index);
    try {
      final header = await File(
        '/proc/self/fd/$fd',
      ).openRead(0, 16).transform(ascii.decoder).join();
      expect(header, 'SQLite format 3\x00');
    } finally {
      await saf.closeFileDescriptor(fd);
    }
    expect(await saf.stat(indexDocument('.redirect'), false), isNull);

    await registry.open(record.recordId);
    final reopened = (await fetchMediaFiles(0, 2)).single;
    expect(reopened.libraryId, track.libraryId);
    expect(reopened.track, track.track);
    await closeLibrary(tester.element(appRoot));
  }, timeout: const Timeout(Duration(minutes: 4)));
}
