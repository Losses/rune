import 'dart:async';
import 'dart:io';

import 'package:file_selector_platform_interface/file_selector_platform_interface.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:rune/main.dart' as app;
import 'package:rune/screens/home/widgets/welcome.dart';
import 'package:rune/screens/library_home/library_home.dart';
import 'package:rune/utils/api/get_media_files_count.dart';
import 'package:rune/utils/dialogs/export_cover_wall/export_cover_wall_dialog.dart';
import 'package:rune/utils/dialogs/export_cover_wall/show_export_cover_wall_dialog.dart';

import 'e2e_library_flow_test.dart' as flow;

class _SavePicker extends FileSelectorPlatform {
  late Completer<FileSaveLocation?> selection;
  int requests = 0;

  @override
  Future<FileSaveLocation?> getSaveLocation({
    List<XTypeGroup>? acceptedTypeGroups,
    SaveDialogOptions options = const SaveDialogOptions(),
  }) {
    requests++;
    return selection.future;
  }
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  // Run with a fresh Linux desktop profile, as in scripts/e2e_library_flow.sh.
  testWidgets('export recovers for empty and artwork-free libraries', (
    tester,
  ) async {
    tester.platformDispatcher.localesTestValue = const [Locale('en')];
    addTearDown(tester.platformDispatcher.clearLocalesTestValue);
    final workspace = await Directory.systemTemp.createTemp('rune-cover-wall-');
    addTearDown(() => workspace.delete(recursive: true));
    final empty = await Directory.fromUri(
      workspace.uri.resolve('Empty'),
    ).create();
    final music = await Directory.fromUri(
      workspace.uri.resolve('Music'),
    ).create();
    final track = await rootBundle.load('assets/startup_0.ogg');
    await File.fromUri(
      music.uri.resolve('startup_0.ogg'),
    ).writeAsBytes(Uint8List.sublistView(track));
    final output = File.fromUri(workspace.uri.resolve('wall.png'));
    final systemPicker = FileSelectorPlatform.instance;
    addTearDown(() => FileSelectorPlatform.instance = systemPicker);
    final picker = _SavePicker();
    FileSelectorPlatform.instance = picker;

    app.main(const []);
    await flow.expectVisible(tester, find.byType(WelcomePage));
    for (final (folder, tracks) in [(music, 1), (empty, 0)]) {
      if (tracks == 0) await flow.openLibrarySettings(tester);
      await flow.addLibrary(tester, folder.path, expectLibraryHome: true);
      await flow.expectAbsent(
        tester,
        find.byType(ContentDialog, skipOffstage: false),
      );
      expect(await getMediaFilesCount(), tracks);

      showExportCoverWallDialog(
        tester.element(find.byType(LibraryHomePage)),
        const [('lib::directory.deep', '/')],
        'wall',
      );
      final dialog = find.byType(ExportCoverWallDialog);
      await flow.expectVisible(tester, dialog);
      final labels = flow.localized(tester);
      final save = find.widgetWithText(FilledButton, labels.save);
      final cancel = find.widgetWithText(Button, labels.cancel);
      final onSave = tester.widget<FilledButton>(save).onPressed!;
      final expectedRequests = picker.requests + 1;
      picker.selection = Completer<FileSaveLocation?>();
      await tester.tap(save);
      await flow.waitFor(
        tester,
        () => picker.requests == expectedRequests,
        description: 'the save chooser to open',
      );
      expect(tester.widget<Button>(cancel).onPressed, isNotNull);
      onSave();
      picker.selection.complete(FileSaveLocation(output.path));
      await flow.expectVisible(
        tester,
        find.text('No cover art is available to export.'),
      );
      expect(
        tester
            .widget<FilledButton>(
              find.widgetWithText(FilledButton, labels.save),
            )
            .onPressed,
        isNotNull,
      );
      expect(picker.requests, expectedRequests);
      expect(await output.exists(), isFalse);
      expect(tester.widget<Button>(cancel).onPressed, isNotNull);
      await tester.tap(cancel);
      await flow.expectAbsent(tester, dialog);
    }
  }, timeout: const Timeout(Duration(minutes: 4)));
}
