import 'dart:io';

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
import 'package:rune/utils/dialogs/file_dialog/file_dialog_window.dart';

import 'e2e_library_flow_test.dart' as flow;

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
      // The export dialog's own buttons; Rune's save dialog has a Save too.
      final save = find.descendant(
        of: dialog,
        matching: find.widgetWithText(FilledButton, labels.save),
      );
      final cancel = find.descendant(
        of: dialog,
        matching: find.widgetWithText(Button, labels.cancel),
      );
      final onSave = tester.widget<FilledButton>(save).onPressed!;
      final chooser = find.byType(FileDialogWindow);
      await tester.tap(save);
      await flow.expectVisible(
        tester,
        chooser,
        description: 'the save chooser to open',
      );
      expect(tester.widget<Button>(cancel).onPressed, isNotNull);
      // A second press while the chooser is open opens no second chooser.
      onSave();
      await tester.pump(const Duration(milliseconds: 300));
      expect(chooser, findsOneWidget);
      final name = find
          .descendant(of: chooser, matching: find.byType(TextBox))
          .last;
      await tester.enterText(name, output.path);
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await flow.expectAbsent(tester, chooser);
      await flow.expectVisible(
        tester,
        find.text('No cover art is available to export.'),
      );
      expect(tester.widget<FilledButton>(save).onPressed, isNotNull);
      expect(await output.exists(), isFalse);
      expect(tester.widget<Button>(cancel).onPressed, isNotNull);
      await tester.tap(cancel);
      await flow.expectAbsent(tester, dialog);
    }
  }, timeout: const Timeout(Duration(minutes: 4)));
}
