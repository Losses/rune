import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:permission_handler/permission_handler.dart';
import 'package:provider/provider.dart';
import 'package:rune/main.dart' as app;
import 'package:rune/providers/library_manager.dart';
import 'package:rune/providers/library_open_state.dart';
import 'package:rune/providers/local_folder_picker.dart';
import 'package:rune/screens/home/widgets/welcome.dart';
import 'package:rune/screens/library_home/library_home.dart';
import 'package:rune/utils/l10n.dart';
import 'package:saf_util/saf_util.dart';

import 'e2e_library_flow_test.dart' as lifecycle;

/// Opens the fixture through DocumentsUI and waits for Rune's initial scan.
Future<LibraryOpenStateProvider> openGrantedFixture(
  WidgetTester tester,
  String treeId, {
  Future<void> Function()? prepareFixture,
}) async {
  const authority = 'com.android.externalstorage.documents';
  app.main(const []);
  final welcome = find.byType(WelcomePage);
  await lifecycle.expectVisible(tester, welcome);
  final context = tester.element(welcome);
  final labels = S.of(context);
  final library = context.read<LibraryOpenStateProvider>();
  final manager = context.read<LibraryManagerProvider>();
  // The host confirms DocumentsUI. Give the real picker the fresh fixture
  // folder as its starting location, independently of its previous selection.
  context.read<LocalFolderPickerProvider>().pickDirectory = (_) async {
    final picked = await SafUtil().pickDirectory(
      initialUri: Uri(
        scheme: 'content',
        host: authority,
        pathSegments: ['document', treeId],
      ).toString(),
      persistablePermission: true,
      writePermission: true,
    );
    return picked?.uri;
  };
  // The host grants the app-op when this test APK starts.
  await Stream<void>.periodic(const Duration(milliseconds: 250))
      .asyncMap((_) => Permission.manageExternalStorage.isGranted)
      .where((granted) => granted)
      .timeout(const Duration(seconds: 30))
      .first;
  await prepareFixture?.call();

  await tapWhenReady(tester, find.text(labels.addLibrary));
  await tapWhenReady(tester, find.text(labels.localFolder));
  await tapWhenReady(tester, find.widgetWithText(Button, labels.confirm));
  await tapWhenReady(tester, find.text(labels.localMode));
  await lifecycle.waitFor(
    tester,
    () => manager.getScanTaskProgress(library.openRecord?.recordId) != null,
    description: 'the initial scan to start for the selected SAF library',
  );
  final granted = Uri.parse(library.openLocalFolderPath!);
  expect(granted.scheme, 'content');
  expect(granted.authority, authority);
  expect(granted.pathSegments, ['tree', treeId, 'document', treeId]);
  expect(await SafUtil().hasPersistedPermission(granted.toString()), isTrue);
  expect(
    await manager.waitForScanToComplete(library.openRecord!.recordId),
    TaskStatus.finished,
    reason: 'the initial scan must finish for the selected SAF library',
  );
  await lifecycle.expectVisible(tester, find.byType(LibraryHomePage));

  return library;
}

/// Taps a visible, enabled target after scrolling it into view.
Future<void> tapWhenReady(WidgetTester tester, Finder matches) async {
  await lifecycle.expectVisible(
    tester,
    matches,
    description: '$matches to appear',
  );
  final target = matches.last;
  await tester.ensureVisible(target);
  await lifecycle.waitFor(
    tester,
    () => target.hitTestable().evaluate().isNotEmpty,
    description: '$target to accept a tap',
  );
  await tester.tap(target.hitTestable());
}
