import 'dart:async';
import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
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
import 'package:rune/utils/dialogs/file_dialog/widgets/items_view.dart';
import 'package:rune/utils/dialogs/file_dialog/widgets/navigation_pane.dart';
import 'package:rune/utils/settings_manager.dart';
import 'package:rune/utils/storage_key_manager.dart';

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
  ],
  for (var index = 1; index <= 40; index++)
    '/music/Album ${index.toString().padLeft(2, '0')}': [
      _entry('/music/Album $index', 'Intro.flac', FileDialogEntryKind.file),
    ],
};

final _backend = FileDialogBackend(
  listPlaces: () async => ListFileDialogPlacesResponse(
    pinned: const [],
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
  listFolder: (location) async => _folders[location.path] ?? const [],
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

void main() {
  late Directory storage;
  setUpAll(() async {
    storage = await Directory.systemTemp.createTemp('rune-dialog-window');
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('plugins.flutter.io/path_provider'),
          (_) async => storage.path,
        );
    StorageKeyManager.initialize(null);
    await SettingsManager().getValue<String>('file-dialog-window-test');
  });
  tearDownAll(() async {
    if (!Platform.isWindows) await storage.delete(recursive: true);
  });
  setUp(() async {
    for (final key in [
      'file_dialog_view_mode',
      'file_dialog_sort',
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
        for (final mode in FileDialogViewMode.values) {
          controller.setViewMode(mode);
          await tester.pumpAndSettle();
          // A layout overflow fails the test through FlutterError.
          expect(_text('Album 01'), findsWidgets, reason: mode.name);
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
      for (final mode in FileDialogViewMode.values) {
        controller.setViewMode(mode);
        controller.clearSelection();
        await tester.pumpAndSettle();
        await tester.longPress(_text('Album 02'));
        await tester.pumpAndSettle();
        // Rename is on the item's menu only.
        expect(_text('Rename'), findsOneWidget, reason: mode.name);
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
}
