import 'dart:io';
import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/main.dart' as app;
import 'package:rune/screens/home/widgets/welcome.dart';
import 'package:rune/utils/dialogs/file_dialog/file_dialog.dart';
import 'package:rune/utils/dialogs/file_dialog/file_dialog_window.dart';
import 'package:rune/utils/dialogs/file_dialog/widgets/address_bar.dart';
import 'package:rune/utils/dialogs/file_dialog/widgets/items_view.dart';

import 'e2e_library_flow_test.dart' as flow;

/// Where screenshots go when the run asks for them, for reviewing the look.
final String? _shots = Platform.environment['FILE_DIALOG_SHOTS'];

Future<void> _shot(WidgetTester tester, String name) async {
  final folder = _shots;
  if (folder == null) return;
  await tester.pump(const Duration(milliseconds: 300));
  RenderObject? boundary = tester.renderObject(find.byType(FileDialogWindow));
  while (boundary != null && !boundary.isRepaintBoundary) {
    boundary = boundary.parent;
  }
  final layer = boundary?.debugLayer;
  if (boundary is! RenderBox || layer is! OffsetLayer) return;
  final image = await layer.toImage(Offset.zero & boundary.size);
  final png = await image.toByteData(format: ui.ImageByteFormat.png);
  image.dispose();
  await File('$folder/$name.png').writeAsBytes(png!.buffer.asUint8List());
}

Future<void> _chord(
  WidgetTester tester,
  LogicalKeyboardKey key, {
  bool control = false,
  bool shift = false,
  bool alt = false,
}) async {
  final modifiers = [
    if (control) LogicalKeyboardKey.controlLeft,
    if (shift) LogicalKeyboardKey.shiftLeft,
    if (alt) LogicalKeyboardKey.altLeft,
  ];
  for (final modifier in modifiers) {
    await tester.sendKeyDownEvent(modifier);
  }
  await tester.sendKeyEvent(key);
  for (final modifier in modifiers.reversed) {
    await tester.sendKeyUpEvent(modifier);
  }
  await tester.pump();
}

Finder _item(String name) => find.descendant(
  of: find.byType(FileDialogItemsView),
  matching: find.text(name),
);

/// Items take mouse clicks: a touch on a folder opens it at once.
Future<void> _click(WidgetTester tester, Finder finder) async {
  await tester.tap(finder, kind: PointerDeviceKind.mouse);
  await tester.pump();
}

Future<void> _doubleClick(WidgetTester tester, Finder finder) async {
  await tester.tap(finder, kind: PointerDeviceKind.mouse);
  await tester.pump(const Duration(milliseconds: 60));
  await tester.tap(finder, kind: PointerDeviceKind.mouse);
  await tester.pump();
}

Future<void> _submitText(WidgetTester tester, Finder box, String text) async {
  await tester.enterText(box, text);
  await tester.testTextInput.receiveAction(TextInputAction.done);
  await tester.pump();
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('the file dialog browses, changes and picks through the hub', (
    tester,
  ) async {
    tester.platformDispatcher.localesTestValue = const [Locale('en')];
    addTearDown(tester.platformDispatcher.clearLocalesTestValue);
    final workspace = await Directory.systemTemp.createTemp('rune-dialog-');
    addTearDown(() => workspace.delete(recursive: true));
    final track = await rootBundle.load('assets/startup_0.ogg');
    final picture = await rootBundle.load('assets/watermark.png');
    Future<void> write(String path, ByteData data) => File(
      '${workspace.path}/$path',
    ).writeAsBytes(Uint8List.sublistView(data));
    await Directory(
      '${workspace.path}/Albums/Live 2019',
    ).create(recursive: true);
    await Directory('${workspace.path}/Album 10').create();
    await Directory('${workspace.path}/Album 2').create();
    await write('track.ogg', track);
    await write('Albums/song.ogg', track);
    await write('cover.png', picture);
    await File('${workspace.path}/notes.txt').writeAsString('liner notes');
    await File('${workspace.path}/.secret.ogg').writeAsString('hidden');

    app.main(const []);
    await flow.expectVisible(tester, find.byType(WelcomePage));
    final labels = flow.localized(tester);
    final root = FileDialogLocation(
      root: const FileDialogRootFileSystem(),
      path: workspace.path,
    );
    final picked = showFileDialog(
      tester.element(find.byType(WelcomePage)),
      FileDialogOptions(
        mode: FileDialogMode.openFile,
        initialFolder: root,
        fileTypes: const [
          FileDialogFileType('Audio', ['ogg']),
          FileDialogFileType.all('All Files'),
        ],
      ),
    );
    await flow.expectVisible(tester, _item('track.ogg'));

    // Folders first in reading order, files of the chosen type, no hidden
    // files.
    final albumTwo = tester.getTopLeft(_item('Album 2')).dy;
    final albumTen = tester.getTopLeft(_item('Album 10')).dy;
    expect(albumTwo, lessThan(albumTen));
    expect(
      tester.getTopLeft(_item('Albums')).dy,
      lessThan(tester.getTopLeft(_item('track.ogg')).dy),
    );
    expect(_item('notes.txt'), findsNothing);
    expect(_item('.secret.ogg'), findsNothing);
    await _shot(tester, '01-details');

    // Every layout, by its shortcut, shows the same items.
    for (final (index, name) in [
      'extra-large',
      'large',
      'medium',
      'small',
      'list',
      'details',
      'tiles',
      'content',
    ].indexed) {
      await _chord(
        tester,
        LogicalKeyboardKey(LogicalKeyboardKey.digit1.keyId + index),
        control: true,
        shift: true,
      );
      await flow.expectVisible(tester, _item('track.ogg'));
      await _shot(tester, '02-view-${index + 1}-$name');
    }
    await _chord(tester, LogicalKeyboardKey.digit6, control: true, shift: true);

    // A double click opens a folder; Alt+Left comes back to it selected.
    await _doubleClick(tester, _item('Albums'));
    await flow.expectVisible(tester, _item('Live 2019'));
    expect(_item('song.ogg'), findsOneWidget);
    await _chord(tester, LogicalKeyboardKey.arrowLeft, alt: true);
    await flow.expectVisible(tester, _item('track.ogg'));

    // Ctrl+L turns the address bar into a box; a relative path goes in.
    await _chord(tester, LogicalKeyboardKey.keyL, control: true);
    final address = find.descendant(
      of: find.byType(FileDialogAddressBar),
      matching: find.byType(TextBox),
    );
    await flow.expectVisible(tester, address);
    await _shot(tester, '03-address-editing');
    await _submitText(tester, address, 'Albums/Live 2019');
    await flow.expectVisible(tester, find.text(labels.fileDialogEmptyFolder));
    await _shot(tester, '04-empty-folder');
    await _chord(tester, LogicalKeyboardKey.arrowUp, alt: true);
    await flow.expectVisible(tester, _item('song.ogg'));

    // A new folder starts renamed in place; the typed name lands on disk.
    await _chord(tester, LogicalKeyboardKey.keyN, control: true, shift: true);
    final rename = find.descendant(
      of: find.byType(FileDialogItemsView),
      matching: find.byType(TextBox),
    );
    await flow.expectVisible(tester, rename);
    await _shot(tester, '05-new-folder-rename');
    await _submitText(tester, rename, 'Rips');
    await flow.expectVisible(tester, _item('Rips'));
    expect(Directory('${workspace.path}/Albums/Rips').existsSync(), isTrue);
    expect(
      Directory(
        '${workspace.path}/Albums/${labels.fileDialogNewFolder}',
      ).existsSync(),
      isFalse,
    );

    // Renaming onto a taken name is refused and explained.
    await _click(tester, _item('Rips'));
    await tester.sendKeyEvent(LogicalKeyboardKey.f2);
    await flow.expectVisible(tester, rename);
    await _submitText(tester, rename, 'Live 2019');
    await flow.expectVisible(
      tester,
      find.text(labels.fileDialogAlreadyExists('Live 2019')),
    );
    await _shot(tester, '06-name-taken');
    await tester.tap(find.widgetWithText(FilledButton, labels.close));
    await tester.pump(const Duration(milliseconds: 300));
    // The name can be corrected right away: the box has the focus back.
    await _shot(tester, '06b-after-explanation');
    final renameBox = tester.widget<TextBox>(rename);
    expect(renameBox.focusNode?.hasFocus, isTrue);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pump();
    expect(find.byType(FileDialogWindow), findsOneWidget);
    expect(rename, findsNothing);
    expect(Directory('${workspace.path}/Albums/Rips').existsSync(), isTrue);

    // Back at the top, the file type box lets every file through.
    await _chord(tester, LogicalKeyboardKey.arrowUp, alt: true);
    await flow.expectVisible(tester, _item('track.ogg'));
    await tester.tap(find.byType(ComboBox<int>));
    await tester.pumpAndSettle();
    await tester.tap(find.text('All Files (*.*)').last);
    await flow.expectVisible(tester, _item('notes.txt'));

    // A name that is not there is explained, not picked.
    final fileName = find
        .descendant(
          of: find.byType(FileDialogWindow),
          matching: find.byType(TextBox),
        )
        .last;
    await _submitText(tester, fileName, 'missing.ogg');
    await flow.expectVisible(
      tester,
      find.text(labels.fileDialogFileNotFound('missing.ogg')),
    );
    await tester.tap(find.widgetWithText(FilledButton, labels.close));
    await tester.pump();

    // Clicking a file puts its name in the box; Open picks it.
    await _click(tester, _item('track.ogg'));
    expect(tester.widget<TextBox>(fileName).controller!.text, 'track.ogg');
    await _shot(tester, '07-file-selected');
    await tester.tap(find.widgetWithText(FilledButton, labels.fileDialogOpen));
    final result = await picked;
    expect(result?.locations, [
      FileDialogLocation(
        root: const FileDialogRootFileSystem(),
        path: '${workspace.path}/track.ogg',
      ),
    ]);
    expect(result?.fileTypeIndex, 1);
    await flow.expectAbsent(tester, find.byType(FileDialogWindow));

    // Saving: the type's extension is added, and an existing file is only
    // replaced after asking.
    await File('${workspace.path}/mix.m3u8').writeAsString('#EXTM3U');
    Future<FileDialogResult?> save() => showFileDialog(
      tester.element(find.byType(WelcomePage)),
      FileDialogOptions(
        mode: FileDialogMode.saveFile,
        initialFolder: root,
        suggestedName: 'Road Trip',
        fileTypes: const [
          FileDialogFileType('Playlists', ['m3u8']),
        ],
      ),
    );

    final first = save();
    await flow.expectVisible(tester, _item('mix.m3u8'));
    await _shot(tester, '08-save');
    await tester.tap(find.widgetWithText(FilledButton, labels.save));
    expect((await first)?.location.path, '${workspace.path}/Road Trip.m3u8');
    // The closing dialog still shows its items for a moment.
    await flow.expectAbsent(tester, find.byType(FileDialogWindow));

    final second = save();
    await flow.expectVisible(tester, _item('mix.m3u8'));
    final saveName = find
        .descendant(
          of: find.byType(FileDialogWindow),
          matching: find.byType(TextBox),
        )
        .last;
    await _submitText(tester, saveName, 'mix');
    await flow.expectVisible(
      tester,
      find.text(labels.fileDialogReplacePrompt('mix.m3u8')),
    );
    await _shot(tester, '09-confirm-replace');
    await tester.tap(
      find.widgetWithText(FilledButton, labels.fileDialogReplace),
    );
    expect((await second)?.location.path, '${workspace.path}/mix.m3u8');
  });
}
