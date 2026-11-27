import 'dart:async';
import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/config/shortcuts.dart' as config;
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/utils/dialogs/file_dialog/file_dialog.dart';
import 'package:rune/utils/dialogs/file_dialog/file_dialog_controller.dart';
import 'package:rune/utils/dialogs/file_dialog/file_dialog_model.dart';
import 'package:rune/utils/dialogs/file_dialog/file_dialog_thumbnails.dart';
import 'package:rune/utils/dialogs/file_dialog/file_dialog_window.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:rune/utils/dialogs/file_dialog/widgets/address_bar.dart';
import 'package:rune/utils/dialogs/file_dialog/widgets/item_icon.dart';
import 'package:rune/utils/dialogs/file_dialog/widgets/items_view.dart';
import 'package:rune/utils/dialogs/file_dialog/widgets/navigation_pane.dart';
import 'package:rune/utils/settings_manager.dart';
import 'package:rune/utils/storage_key_manager.dart';
import '../settings_folder.dart';

/// Breakpoints from the window width, as the real provider finds them.
class _Responsive extends ChangeNotifier implements ResponsiveProvider {
  _Responsive(this.width);

  final double width;

  DeviceType get _type => const [
    DeviceType.zune,
    DeviceType.phone,
    DeviceType.mobile,
    DeviceType.tablet,
    DeviceType.desktop,
    DeviceType.tv,
  ].firstWhere((type) => width <= type.end);

  @override
  bool smallerOrEqualTo(DeviceType type, [bool autoOrientation = true]) =>
      _type.start <= type.start;

  @override
  DeviceType getActiveDeviceType(List<DeviceType> deviceTypes) =>
      deviceTypes.contains(_type) ? _type : DeviceType.tv;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

const _longName =
    'An extraordinarily long name for a recording, which must show whole.flac';

FileDialogLocation _at(String path) =>
    FileDialogLocation(root: const FileDialogRootFileSystem(), path: path);

FileDialogEntry _entry(String parent, String name, FileDialogEntryKind kind) =>
    FileDialogEntry(
      name: name,
      location: _at('$parent/$name'),
      kind: kind,
      size: Uint64.fromBigInt(BigInt.from(4096)),
      modified: Uint64.fromBigInt(BigInt.from(1795000000)),
      hidden: false,
    );

final _folders = <String, List<FileDialogEntry>>{
  '/music': [
    for (var index = 1; index <= 40; index++)
      _entry(
        '/music',
        'Album ${index.toString().padLeft(2, '0')}',
        FileDialogEntryKind.folder,
      ),
    for (var index = 1; index <= 10; index++)
      _entry('/music', 'Track $index.flac', FileDialogEntryKind.file),
  ],
  '/picks': [
    for (var index = 1; index <= 3; index++)
      _entry('/picks', 'Track $index.flac', FileDialogEntryKind.file),
    _entry('/picks', _longName, FileDialogEntryKind.file),
  ],
  for (var index = 1; index <= 40; index++)
    '/music/Album ${index.toString().padLeft(2, '0')}': [
      _entry('/music/Album $index', 'Intro.flac', FileDialogEntryKind.file),
    ],
};

/// How many folders the dialog has listed, for telling a refresh happened.
var _listings = 0;

final _backend = FileDialogBackend(
  listPlaces: () async => ListFileDialogPlacesResponse(
    pinned: [
      FileDialogPlace(
        location: _at('/picks'),
        name: const FileDialogPlaceNameCustom(name: 'Favourites'),
      ),
    ],
    folders: [
      FileDialogPlace(
        location: _at('/music'),
        name: const FileDialogPlaceNameKnown(
          folder: FileDialogKnownFolder.music,
        ),
      ),
    ],
    volumes: const [],
  ),
  listFolder: (location) async {
    _listings++;
    return _folders[location.path] ?? const [];
  },
  resolve: (typed, current) async => ResolveFileDialogPathResponse(
    location: _at('${current.path}/$typed'),
    target: FileDialogPathTarget.missing,
  ),
  createFolder: (parent, name) async => _at('${parent.path}/$name'),
  rename: (location, name) async => location,
);

/// Intents Rune's own shortcuts reach; while the dialog has the keyboard,
/// none should.
class _Recorder extends Action<Intent> {
  final reached = <Intent>[];

  @override
  Object? invoke(Intent intent) {
    reached.add(intent);
    return null;
  }
}

class _Harness {
  final closed = <FileDialogResult?>[];
  final recorder = _Recorder();

  FileDialogController controller(WidgetTester tester) => tester
      .state<FileDialogWindowState>(find.byType(FileDialogWindow))
      .controller;

  Future<void> pump(
    WidgetTester tester,
    Size size, {
    FileDialogMode mode = FileDialogMode.openFile,
  }) async {
    tester.view.physicalSize = size;
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final thumbnails = FileDialogThumbnails(fetch: (_, _) async => null);
    addTearDown(thumbnails.dispose);
    await tester.pumpWidget(
      ChangeNotifierProvider<ResponsiveProvider>.value(
        value: _Responsive(size.width),
        child: FluentApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          // Rune's shortcuts sit above the navigator, as in main.dart.
          builder: (context, child) => Shortcuts(
            shortcuts: config.shortcuts,
            child: Actions(
              actions: {
                for (final intent in config.shortcuts.values)
                  intent.runtimeType: recorder,
              },
              child: child!,
            ),
          ),
          home: FileDialogWindow(
            options: FileDialogOptions(
              mode: mode,
              initialFolder: _at('/music'),
              fileTypes: const [
                FileDialogFileType('Audio', ['flac']),
              ],
            ),
            close: closed.add,
            backend: _backend,
            thumbnails: thumbnails,
          ),
        ),
      ),
    );
    // Settings were read once outside the test's fake clock; their futures
    // finish on the real event loop, so the dialog's start runs both.
    final controller = this.controller(tester);
    for (var turn = 0; turn < 50; turn++) {
      if (controller.hasEntries && controller.places != null) break;
      await tester.runAsync(() => Future<void>.delayed(Duration.zero));
      await tester.pump();
    }
    expect(controller.hasEntries, isTrue);
    await tester.pumpAndSettle();
  }
}

Finder _text(String text) => find.text(text);

/// An item's name in the item area (the file name box may show it too).
Finder _item(String name) => find.descendant(
  of: find.byType(FileDialogItemsView),
  matching: find.text(name),
);

/// A Fluent tooltip, which flutter_test's byTooltip does not know.
Finder _tooltip(String message) => find.byWidgetPredicate(
  (widget) => widget is Tooltip && widget.message == message,
);

void main() {
  late Directory storage;
  setUpAll(() async {
    storage = await emptySettingsFolder('rune-dialog-window');
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('plugins.flutter.io/path_provider'),
          (_) async => storage.path,
        );
    StorageKeyManager.initialize(null);
    await SettingsManager().getValue<String>('file-dialog-window-test');
  });
  setUp(() async {
    for (final key in [
      'file_dialog_view',
      'file_dialog_columns',
      'file_dialog_navigation_share',
      'file_dialog_sort',
      'file_dialog_show_hidden',
      'file_dialog_last_folder',
    ]) {
      await SettingsManager().removeValue(key);
    }
  });

  group('every layout fits', () {
    for (final (name, size) in [
      ('a desktop window', const Size(1280, 800)),
      ('a tablet', const Size(720, 1000)),
      ('a phone', const Size(400, 760)),
      ('a short window', const Size(1000, 480)),
      // The frame a dialog that is already open keeps when the window
      // shrinks further.
      ('the smallest dialog frame', const Size(320, 480)),
    ]) {
      testWidgets('on $name', (tester) async {
        final harness = _Harness();
        await harness.pump(tester, size);
        final controller = harness.controller(tester);
        // The named views, and icons between two of them.
        for (final view in [
          ...FileDialogView.named,
          FileDialogView.icons(150),
        ]) {
          controller.setView(view);
          await tester.pumpAndSettle();
          // A layout overflow fails the test through FlutterError.
          expect(_text('Album 01'), findsWidgets, reason: '$view');
        }
        final narrow = size.width <= DeviceType.tablet.end;
        expect(
          find.byType(FileDialogNavigationPane),
          narrow ? findsNothing : findsOneWidget,
        );
        // The file name box and the address bar stay wide enough to use.
        final boxes = find.descendant(
          of: find.byType(FileDialogWindow),
          matching: find.byType(TextBox),
        );
        expect(tester.getSize(boxes.last).width, greaterThan(150));
      });
    }
  });

  group('touch', () {
    testWidgets('a drag that starts on a folder scrolls without opening it', (
      tester,
    ) async {
      final harness = _Harness();
      await harness.pump(tester, const Size(1280, 800));
      final before = tester.getTopLeft(_text('Album 05'));
      await tester.dragFrom(
        tester.getCenter(_text('Album 05')),
        const Offset(0, -200),
      );
      await tester.pumpAndSettle();
      expect(harness.controller(tester).current, _at('/music'));
      expect(harness.controller(tester).selection, isEmpty);
      expect(tester.getTopLeft(_text('Album 05')).dy, lessThan(before.dy));
    });

    testWidgets('a tap opens a folder, and rows grow to finger size', (
      tester,
    ) async {
      final harness = _Harness();
      await harness.pump(tester, const Size(1280, 800));
      final mouseRow =
          tester.getTopLeft(_text('Album 02')).dy -
          tester.getTopLeft(_text('Album 01')).dy;
      await tester.tap(_text('Album 03'));
      await tester.pumpAndSettle();
      expect(harness.controller(tester).current, _at('/music/Album 03'));
      expect(_text('Intro.flac'), findsOneWidget);
      await harness.controller(tester).back();
      await tester.pumpAndSettle();
      final touchRow =
          tester.getTopLeft(_text('Album 02')).dy -
          tester.getTopLeft(_text('Album 01')).dy;
      expect(mouseRow, 28);
      expect(touchRow, 40);
    });

    testWidgets('a finger hides the keyboard outline and a key shows it', (
      tester,
    ) async {
      final harness = _Harness();
      await harness.pump(tester, const Size(1280, 800));
      await harness.controller(tester).navigate(_at('/picks'));
      await tester.pumpAndSettle();
      bool outlined() => tester
          .widgetList<FocusBorder>(find.byType(FocusBorder))
          .any((border) => border.focused);
      await tester.tap(_text('Track 2.flac'));
      await tester.pumpAndSettle();
      expect(harness.controller(tester).focused, _at('/picks/Track 2.flac'));
      expect(outlined(), isFalse);
      // Any key, even one that moves nothing, brings the outline back.
      await tester.sendKeyEvent(LogicalKeyboardKey.shiftLeft);
      await tester.pumpAndSettle();
      expect(outlined(), isTrue);
    });

    testWidgets('taps pick several files where several can be picked', (
      tester,
    ) async {
      final harness = _Harness();
      await harness.pump(
        tester,
        const Size(1280, 800),
        mode: FileDialogMode.openFiles,
      );
      final controller = harness.controller(tester);
      await controller.navigate(_at('/picks'));
      await tester.pumpAndSettle();
      await tester.tap(_text('Track 1.flac'));
      await tester.pump();
      await tester.tap(_text('Track 2.flac'));
      await tester.pump();
      expect(controller.selection, {
        _at('/picks/Track 1.flac'),
        _at('/picks/Track 2.flac'),
      });
      expect(controller.fileName.text, '"Track 1.flac" "Track 2.flac"');
      await tester.tap(_text('Track 1.flac'));
      await tester.pump();
      expect(controller.selection, {_at('/picks/Track 2.flac')});
    });

    testWidgets('a long press opens the item menu in every view', (
      tester,
    ) async {
      final harness = _Harness();
      await harness.pump(tester, const Size(1280, 800));
      final controller = harness.controller(tester);
      for (final view in FileDialogView.named) {
        controller.setView(view);
        controller.clearSelection();
        await tester.pumpAndSettle();
        await tester.longPress(_text('Album 02'));
        await tester.pumpAndSettle();
        // Rename is on the item's menu only.
        expect(_text('Rename'), findsOneWidget, reason: '$view');
        expect(controller.selection, {_at('/music/Album 02')});
        expect(controller.current, _at('/music'));
        await tester.sendKeyEvent(LogicalKeyboardKey.escape);
        await tester.pumpAndSettle();
        expect(_text('Rename'), findsNothing);
      }
    });
  });

  group('the keyboard', () {
    /// Every key of Rune's own shortcuts but Escape, which closes the
    /// dialog and is tested on its own.
    final appKeys = [
      for (final activator in config.shortcuts.keys)
        if (activator.trigger != LogicalKeyboardKey.escape) activator,
    ];

    Future<void> pressAppShortcuts(WidgetTester tester) async {
      expect(appKeys, hasLength(greaterThan(10)));
      for (final activator in appKeys) {
        final modifiers = [
          if (activator.control) LogicalKeyboardKey.controlLeft,
          if (activator.shift) LogicalKeyboardKey.shiftLeft,
          if (activator.alt) LogicalKeyboardKey.altLeft,
          if (activator.meta) LogicalKeyboardKey.metaLeft,
        ];
        for (final modifier in modifiers) {
          await tester.sendKeyDownEvent(modifier);
        }
        await tester.sendKeyEvent(
          activator.trigger,
          // Android's back key has no scan code to simulate; web events
          // carry none.
          platform: activator.trigger == LogicalKeyboardKey.goBack
              ? 'web'
              : null,
          physicalKey: activator.trigger == LogicalKeyboardKey.goBack
              ? PhysicalKeyboardKey.browserBack
              : null,
        );
        for (final modifier in modifiers.reversed) {
          await tester.sendKeyUpEvent(modifier);
        }
        await tester.pump();
      }
    }

    testWidgets('reaches none of Rune\'s shortcuts from the items', (
      tester,
    ) async {
      final harness = _Harness();
      await harness.pump(tester, const Size(1280, 800));
      await tester.tap(_text('Album 02'), kind: PointerDeviceKind.mouse);
      await tester.pump();
      await pressAppShortcuts(tester);
      expect(harness.recorder.reached, isEmpty);
    });

    testWidgets('reaches none of them while a menu is open', (tester) async {
      final harness = _Harness();
      await harness.pump(tester, const Size(1280, 800));
      await tester.tap(
        _text('Album 02'),
        buttons: kSecondaryButton,
        kind: PointerDeviceKind.mouse,
      );
      await tester.pumpAndSettle();
      expect(_text('Copy as Path'), findsOneWidget);
      await pressAppShortcuts(tester);
      expect(harness.recorder.reached, isEmpty);
    });

    testWidgets('reaches none of them from the file name box', (tester) async {
      final harness = _Harness();
      await harness.pump(tester, const Size(1280, 800));
      await tester.tap(find.byType(TextBox).last);
      await tester.pump();
      await pressAppShortcuts(tester);
      expect(harness.recorder.reached, isEmpty);
    });

    /// Asks for a file that is not there; the dialog says so and waits.
    Future<void> openMissingFile(WidgetTester tester, _Harness harness) async {
      harness.controller(tester).fileName.text = 'missing.flac';
      unawaited(harness.controller(tester).submit());
      await tester.pumpAndSettle();
      expect(find.textContaining('Cannot find the file'), findsOneWidget);
    }

    testWidgets('reaches none of them while a notice is open', (tester) async {
      final harness = _Harness();
      await harness.pump(tester, const Size(1280, 800));
      await openMissingFile(tester, harness);
      await pressAppShortcuts(tester);
      expect(harness.recorder.reached, isEmpty);
    });

    testWidgets('Escape closes a notice first and the dialog after', (
      tester,
    ) async {
      final harness = _Harness();
      await harness.pump(tester, const Size(1280, 800));
      await openMissingFile(tester, harness);
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(find.textContaining('Cannot find the file'), findsNothing);
      expect(harness.closed, isEmpty);
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(harness.closed, [null]);
      expect(harness.recorder.reached, isEmpty);
    });

    testWidgets('Escape closes a menu first and the dialog after', (
      tester,
    ) async {
      final harness = _Harness();
      await harness.pump(tester, const Size(1280, 800));
      await tester.tap(
        _text('Album 02'),
        buttons: kSecondaryButton,
        kind: PointerDeviceKind.mouse,
      );
      await tester.pumpAndSettle();
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(_text('Copy as Path'), findsNothing);
      expect(harness.closed, isEmpty);
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(harness.closed, [null]);
      expect(harness.recorder.reached, isEmpty);
    });

    testWidgets('the mouse back button goes back a folder', (tester) async {
      final harness = _Harness();
      await harness.pump(tester, const Size(1280, 800));
      await harness.controller(tester).navigate(_at('/music/Album 04'));
      await tester.pumpAndSettle();
      // With a menu open the press belongs to the menu.
      await tester.tap(
        _text('Intro.flac'),
        buttons: kSecondaryButton,
        kind: PointerDeviceKind.mouse,
      );
      await tester.pumpAndSettle();
      await tester.tap(
        _text('Rename'),
        buttons: kBackMouseButton,
        kind: PointerDeviceKind.mouse,
      );
      await tester.pumpAndSettle();
      expect(harness.controller(tester).current, _at('/music/Album 04'));
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(_text('Rename'), findsNothing);
      await tester.tap(
        // The item; the file name box also shows the name now.
        find.descendant(
          of: find.byType(FileDialogItemsView),
          matching: _text('Intro.flac'),
        ),
        buttons: kBackMouseButton,
        kind: PointerDeviceKind.mouse,
      );
      await tester.pumpAndSettle();
      expect(harness.controller(tester).current, _at('/music'));
      expect(harness.closed, isEmpty);
    });
  });

  group('Windows details', () {
    /// A mouse click, as the dialog tells mouse from touch.
    Future<void> click(WidgetTester tester, Finder finder) =>
        tester.tap(finder, kind: PointerDeviceKind.mouse);

    /// Clicks are told apart by the time between them; let real time pass.
    Future<void> wait(WidgetTester tester, int milliseconds) => tester.runAsync(
      () => Future<void>.delayed(Duration(milliseconds: milliseconds)),
    );

    Future<FileDialogController> openPicks(WidgetTester tester) async {
      final harness = _Harness();
      await harness.pump(tester, const Size(1280, 800));
      final controller = harness.controller(tester);
      await controller.navigate(_at('/picks'));
      await tester.pumpAndSettle();
      return controller;
    }

    testWidgets('a second, slow click on a selected name renames it', (
      tester,
    ) async {
      final controller = await openPicks(tester);
      final name = _item('Track 2.flac');
      await click(tester, name);
      await tester.pump();
      await wait(tester, 600);
      await click(tester, name);
      await tester.pump();
      // Nothing yet: a second click could still make it a double click.
      expect(controller.renaming, isNull);
      await tester.pump(const Duration(milliseconds: 600));
      expect(controller.renaming, _at('/picks/Track 2.flac'));
    });

    testWidgets('a slow click followed by a quick one opens instead', (
      tester,
    ) async {
      final harness = _Harness();
      await harness.pump(tester, const Size(1280, 800));
      final controller = harness.controller(tester);
      final folder = _item('Album 03');
      await click(tester, folder);
      await tester.pump();
      await wait(tester, 600);
      await click(tester, folder);
      await tester.pump(const Duration(milliseconds: 50));
      await click(tester, folder);
      await tester.pumpAndSettle();
      expect(controller.current, _at('/music/Album 03'));
      await tester.pump(const Duration(milliseconds: 600));
      expect(controller.renaming, isNull);
    });

    testWidgets('a click on the icon, or with Ctrl, does not rename', (
      tester,
    ) async {
      final controller = await openPicks(tester);
      final name = _item('Track 2.flac');
      await click(tester, name);
      await tester.pump();
      await wait(tester, 600);
      final row = find.ancestor(of: name, matching: find.byType(Row)).first;
      await click(
        tester,
        find.descendant(of: row, matching: find.byType(FileDialogItemIcon)),
      );
      await tester.pump(const Duration(milliseconds: 600));
      expect(controller.renaming, isNull);
      await wait(tester, 600);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await click(tester, name);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pump(const Duration(milliseconds: 600));
      expect(controller.renaming, isNull);
    });

    testWidgets('a click just after another press does not rename', (
      tester,
    ) async {
      final controller = await openPicks(tester);
      final name = _item('Track 2.flac');
      await click(tester, name);
      await tester.pump();
      await wait(tester, 600);
      // A right click, then at once a click on the name: File Explorer
      // takes no click with another just before it for a rename.
      await tester.tap(
        name,
        buttons: kSecondaryButton,
        kind: PointerDeviceKind.mouse,
      );
      await tester.pumpAndSettle();
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      await click(tester, name);
      await tester.pump(const Duration(milliseconds: 600));
      expect(controller.renaming, isNull);
    });

    testWidgets('the renamed text stays where the name was in every view', (
      tester,
    ) async {
      final controller = await openPicks(tester);
      controller.select(_at('/picks/Track 2.flac'));
      // Measured from the item's own frame: the view may scroll to show
      // the box, as File Explorer's does.
      Finder frame() => find
          .ancestor(
            of: find.descendant(
              of: find.byType(FileDialogItemsView),
              matching: find.byWidgetPredicate(
                (widget) =>
                    (widget is Text && widget.data == 'Track 2.flac') ||
                    widget is EditableText,
              ),
            ),
            matching: find.byWidgetPredicate(
              (widget) => widget.runtimeType.toString() == '_ItemFrame',
            ),
          )
          .first;
      for (final view in [...FileDialogView.named, FileDialogView.icons(150)]) {
        controller.setView(view);
        await tester.pumpAndSettle();
        // Where the first letter is, in the name and then in the box.
        final paragraph = tester.renderObject<RenderParagraph>(
          _item('Track 2.flac'),
        );
        final label =
            paragraph.localToGlobal(
              paragraph.getOffsetForCaret(
                const TextPosition(offset: 0),
                Rect.zero,
              ),
            ) -
            tester.getTopLeft(frame());
        controller.startRename();
        await tester.pumpAndSettle();
        final field = find.descendant(
          of: find.byType(FileDialogItemsView),
          matching: find.byType(EditableText),
        );
        final render = tester.state<EditableTextState>(field).renderEditable;
        final start =
            render.localToGlobal(
              render
                  .getLocalRectForCaret(const TextPosition(offset: 0))
                  .topLeft,
            ) -
            tester.getTopLeft(frame());
        expect(start.dx, closeTo(label.dx, 1), reason: '$view');
        expect(start.dy, closeTo(label.dy, 1), reason: '$view');
        controller.cancelRename();
        await tester.pumpAndSettle();
      }
    });

    testWidgets('Ctrl and the mouse wheel zoom; the wheel alone scrolls', (
      tester,
    ) async {
      final harness = _Harness();
      await harness.pump(tester, const Size(1280, 800));
      final controller = harness.controller(tester);
      final pointer = TestPointer(1, PointerDeviceKind.mouse);
      await tester.sendEventToBinding(
        pointer.hover(tester.getCenter(find.byType(FileDialogItemsView))),
      );
      final seen = <FileDialogView>[];
      Future<void> wheel(double dy) async {
        await tester.sendEventToBinding(pointer.scroll(Offset(0, dy)));
        await tester.pump();
        seen.add(controller.view);
      }

      controller.setView(FileDialogView.largeIcons);
      await tester.pumpAndSettle();
      // Ctrl is let go before anything is checked, so a failure here
      // leaves no key held for the tests after it.
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await wheel(50);
      await wheel(-50);
      await wheel(-50);
      controller.setView(FileDialogView.details);
      await tester.pumpAndSettle();
      await wheel(50);
      await wheel(-50);
      await wheel(-50);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      expect(seen[0].layout, FileDialogLayout.icons);
      expect(seen[0].iconSize, lessThan(96));
      expect(seen[2].iconSize, greaterThan(96));
      expect(seen.sublist(3), [
        FileDialogView.tiles,
        FileDialogView.details,
        FileDialogView.list,
      ]);
      controller.setView(FileDialogView.details);
      await tester.pumpAndSettle();
      final before = tester.getTopLeft(_text('Album 05')).dy;
      await wheel(120);
      await tester.pumpAndSettle();
      expect(controller.view, FileDialogView.details);
      expect(tester.getTopLeft(_text('Album 05')).dy, lessThan(before));
      await tester.pump(const Duration(seconds: 1));
    });

    testWidgets('the view button has the fill of the New Folder button', (
      tester,
    ) async {
      await _Harness().pump(tester, const Size(1280, 800));
      // Rendered pixels, as the two buttons paint their fill differently.
      final boundary = tester.renderObject<RenderRepaintBoundary>(
        find
            .ancestor(
              of: find.byType(FileDialogWindow),
              matching: find.byType(RepaintBoundary),
            )
            .first,
      );
      final image = (await tester.runAsync(() => boundary.toImage()))!;
      final bytes = (await tester.runAsync(image.toByteData))!;
      final origin = boundary.localToGlobal(Offset.zero);
      int pixel(Offset at) {
        final local = at - origin;
        final index = (local.dy.round() * image.width + local.dx.round()) * 4;
        return bytes.getUint32(index);
      }

      // Just inside each button's left edge, halfway down: its fill.
      Offset fillOf(Finder button) {
        final rect = tester.getRect(button);
        return Offset(rect.left + 4, rect.center.dy);
      }

      final newFolder = pixel(
        fillOf(find.widgetWithText(Button, 'New Folder')),
      );
      final view = pixel(fillOf(_tooltip('Change Your View')));
      final arrow = pixel(
        tester.getRect(_tooltip('More Options')).centerLeft +
            const Offset(4, 0),
      );
      // The dialog behind them, between the two buttons.
      final behind = pixel(
        Offset(
          tester.getRect(find.widgetWithText(Button, 'New Folder')).right + 40,
          tester.getRect(_tooltip('Change Your View')).center.dy,
        ),
      );
      image.dispose();
      expect(newFolder, isNot(behind));
      expect(view, newFolder);
      expect(arrow, newFolder);
    });

    testWidgets('hidden items are switched under the view slider', (
      tester,
    ) async {
      final harness = _Harness();
      await harness.pump(tester, const Size(1280, 800));
      final controller = harness.controller(tester);
      expect(controller.showHidden, isFalse);
      await click(tester, _tooltip('More Options'));
      await tester.pumpAndSettle();
      final hidden = _text('Hidden Items');
      expect(hidden, findsOneWidget);
      // Below the last view.
      expect(
        tester.getTopLeft(hidden).dy,
        greaterThan(tester.getBottomLeft(_text('Content')).dy),
      );
      expect(find.byIcon(Symbols.check), findsNothing);
      await click(tester, hidden);
      await tester.pumpAndSettle();
      expect(controller.showHidden, isTrue);
      expect(hidden, findsNothing);
      await click(tester, _tooltip('More Options'));
      await tester.pumpAndSettle();
      expect(find.byIcon(Symbols.check), findsOneWidget);
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
    });

    testWidgets('the view button steps and its arrow opens the slider', (
      tester,
    ) async {
      final harness = _Harness();
      await harness.pump(tester, const Size(1280, 800));
      final controller = harness.controller(tester);
      expect(_text('Sort By'), findsNothing);
      await click(tester, _tooltip('Change Your View'));
      await tester.pumpAndSettle();
      expect(controller.view, FileDialogView.tiles);
      await click(tester, _tooltip('More Options'));
      await tester.pumpAndSettle();
      expect(_text('Extra Large Icons'), findsOneWidget);
      // No dot marks the current view; the slider's thumb does.
      expect(find.byIcon(Symbols.fiber_manual_record), findsNothing);
      final track = find.byWidgetPredicate(
        (widget) =>
            widget is CustomPaint &&
            widget.painter.runtimeType.toString() == '_SliderPainter',
      );
      final top = tester.getTopLeft(track);
      // Halfway between Extra large and Large icons.
      await tester.tapAt(top + const Offset(14, 16 + 32 * 0.5));
      await tester.pump();
      expect(controller.view.layout, FileDialogLayout.icons);
      expect(controller.view.iconSize, closeTo(156.8, 0.5));
      await tester.dragFrom(
        top + const Offset(14, 16),
        const Offset(0, 32 * 5),
      );
      await tester.pump();
      expect(controller.view, FileDialogView.details);
      await click(tester, _text('List'));
      await tester.pumpAndSettle();
      expect(controller.view, FileDialogView.list);
      expect(_text('Extra Large Icons'), findsNothing);
    });

    testWidgets('the folder menu marks the current view before its name', (
      tester,
    ) async {
      await openPicks(tester);
      final items = find.byType(FileDialogItemsView);
      await tester.tapAt(
        tester.getBottomRight(items) - const Offset(40, 40),
        buttons: kSecondaryButton,
        kind: PointerDeviceKind.mouse,
      );
      await tester.pumpAndSettle();
      await click(tester, _text('View'));
      await tester.pumpAndSettle();
      final dot = find.byIcon(Symbols.fiber_manual_record);
      expect(dot, findsOneWidget);
      final details = _text('Details');
      expect(tester.getCenter(dot).dx, lessThan(tester.getTopLeft(details).dx));
      expect(
        tester.getCenter(dot).dy,
        closeTo(tester.getCenter(details).dy, 2),
      );
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle(const Duration(seconds: 1));
    });

    testWidgets('the navigation pane is dragged wider and stays so', (
      tester,
    ) async {
      final harness = _Harness();
      await harness.pump(tester, const Size(1280, 800));
      final pane = find.byType(FileDialogNavigationPane);
      final before = tester.getSize(pane).width;
      await tester.dragFrom(
        tester.getTopRight(pane) + const Offset(4, 100),
        const Offset(80, 0),
      );
      await tester.pumpAndSettle();
      expect(tester.getSize(pane).width, closeTo(before + 80, 1));
      await tester.pumpWidget(const SizedBox());
      await _Harness().pump(tester, const Size(1280, 800));
      expect(tester.getSize(pane).width, closeTo(before + 80, 1));
    });

    testWidgets('the pin keeps clear of the pane scroll bar', (tester) async {
      await _Harness().pump(tester, const Size(1280, 800));
      final pin = find.byIcon(Symbols.keep);
      expect(pin, findsOneWidget);
      final pane = find.byType(FileDialogNavigationPane);
      expect(
        tester.getTopRight(pane).dx - tester.getTopRight(pin).dx,
        greaterThanOrEqualTo(16),
      );
    });

    testWidgets('the details header has File Explorer\'s column menu', (
      tester,
    ) async {
      final controller = await openPicks(tester);
      final items = find.byType(FileDialogItemsView);
      Future<void> rightClick(Offset at) async {
        await tester.tapAt(
          at,
          buttons: kSecondaryButton,
          kind: PointerDeviceKind.mouse,
        );
        await tester.pumpAndSettle();
      }

      await rightClick(tester.getCenter(_text('Date Modified')));
      expect(_text('Size Column to Fit'), findsOneWidget);
      expect(_text('Size All Columns to Fit'), findsOneWidget);
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      // Off the columns, only fitting them all is offered.
      await rightClick(tester.getTopLeft(items) + const Offset(4, 14));
      expect(_text('Size Column to Fit'), findsNothing);
      expect(_text('Size All Columns to Fit'), findsOneWidget);
      // The name cannot be switched off; the type can.
      await click(tester, _text('Name').last);
      await tester.pumpAndSettle();
      expect(controller.hiddenColumns, isEmpty);
      await click(tester, _text('Type').last);
      await tester.pumpAndSettle();
      expect(controller.hiddenColumns, {FileDialogColumn.type});
      expect(_text('Type'), findsNothing);
    });

    testWidgets('columns fit their contents and resize by their divider', (
      tester,
    ) async {
      final controller = await openPicks(tester);
      bool cut() => tester
          .renderObject<RenderParagraph>(_item(_longName))
          .didExceedMaxLines;
      controller.setColumnWidth(FileDialogColumn.name, 200);
      await tester.pumpAndSettle();
      expect(cut(), isTrue);
      // The name's divider is on its end, just before the next header's
      // padding. Dragging it resizes the name; the next column keeps its width.
      Offset divider() =>
          tester.getTopLeft(_text('Date Modified')) + const Offset(-11, 14);
      final date = tester.getSize(_text('Date Modified'));
      await tester.dragFrom(divider(), const Offset(60, 0));
      await tester.pumpAndSettle();
      expect(controller.columnWidths[FileDialogColumn.name], 260);
      expect(tester.getSize(_text('Date Modified')), date);
      // Double-clicking it fits the name to its longest value.
      await tester.tapAt(divider(), kind: PointerDeviceKind.mouse);
      await tester.pump(const Duration(milliseconds: 50));
      await tester.tapAt(divider(), kind: PointerDeviceKind.mouse);
      await tester.pumpAndSettle();
      expect(cut(), isFalse);
      final fitted = controller.columnWidths[FileDialogColumn.name]!;
      // The header menu's Size Column to Fit gives the same width.
      controller.setColumnWidth(FileDialogColumn.name, 200);
      await tester.pumpAndSettle();
      await tester.tapAt(
        tester.getCenter(_text('Name')),
        buttons: kSecondaryButton,
        kind: PointerDeviceKind.mouse,
      );
      await tester.pumpAndSettle();
      await click(tester, _text('Size Column to Fit'));
      await tester.pumpAndSettle();
      expect(controller.columnWidths[FileDialogColumn.name], fitted);
      expect(cut(), isFalse);
    });

    testWidgets('the buttons keep their width in a wide dialog', (
      tester,
    ) async {
      await _Harness().pump(tester, const Size(1920, 1080));
      final open = find.widgetWithText(FilledButton, 'Open');
      final cancel = find.widgetWithText(Button, 'Cancel');
      expect(tester.getSize(open).width, lessThan(200));
      expect(tester.getSize(cancel).width, lessThan(200));
      // At the end, lined up with the file type box above them.
      expect(
        tester.getTopRight(cancel).dx,
        closeTo(tester.getTopRight(find.byType(ComboBox<int>)).dx, 1),
      );
    });

    testWidgets('refresh sits in the address bar and stays while typing', (
      tester,
    ) async {
      await _Harness().pump(tester, const Size(1280, 800));
      final bar = find.byType(FileDialogAddressBar);
      final refresh = find.descendant(
        of: bar,
        matching: _tooltip('Refresh (F5)'),
      );
      expect(refresh, findsOneWidget);
      final before = _listings;
      await click(tester, refresh);
      await tester.pumpAndSettle();
      expect(_listings, before + 1);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyL);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pumpAndSettle();
      final state = tester.state<FileDialogAddressBarState>(bar);
      expect(state.editing, isTrue);
      expect(refresh, findsOneWidget);
      await tester.enterText(
        find.descendant(of: bar, matching: find.byType(EditableText)),
        '/somewhere',
      );
      await click(tester, refresh);
      await tester.pumpAndSettle();
      expect(_listings, before + 2);
      expect(state.editing, isTrue);
      expect(_text('/somewhere'), findsOneWidget);
    });
  });
}
