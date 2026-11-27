import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/utils/api/request_failure.dart';
import 'package:rune/utils/dialogs/file_dialog/file_dialog.dart';
import 'package:rune/utils/dialogs/file_dialog/file_dialog_controller.dart';
import 'package:rune/utils/dialogs/file_dialog/file_dialog_model.dart';
import 'package:rune/utils/settings_manager.dart';
import 'package:rune/utils/storage_key_manager.dart';

FileDialogLocation _at(String path) =>
    FileDialogLocation(root: const FileDialogRootFileSystem(), path: path);

FileDialogEntry _folder(String parent, String name, {bool hidden = false}) =>
    FileDialogEntry(
      name: name,
      location: _at('$parent/$name'),
      kind: FileDialogEntryKind.folder,
      size: Uint64.fromBigInt(BigInt.zero),
      modified: null,
      hidden: hidden,
    );

FileDialogEntry _file(
  String parent,
  String name, {
  int size = 1,
  bool hidden = false,
}) => FileDialogEntry(
  name: name,
  location: _at('$parent/$name'),
  kind: FileDialogEntryKind.file,
  size: Uint64.fromBigInt(BigInt.from(size)),
  modified: null,
  hidden: hidden,
);

/// A file system in memory: folders by path, each with its entries.
class _FakeFiles {
  _FakeFiles(this.folders);

  final Map<String, List<FileDialogEntry>> folders;
  final created = <String>[];
  final renamed = <(String, String)>[];

  RequestFailure _failure(FileDialogFailure failure) => RequestFailure(
    'Test',
    RequestErrorKind.notFound,
    '',
    RequestFailureCauseFileDialogFailed(failure: failure),
  );

  FileDialogBackend get backend => FileDialogBackend(
    listPlaces: () async => ListFileDialogPlacesResponse(
      pinned: const [],
      folders: [
        FileDialogPlace(
          location: _at('/home'),
          name: const FileDialogPlaceNameKnown(
            folder: FileDialogKnownFolder.home,
          ),
        ),
      ],
      volumes: const [],
    ),
    listFolder: (location) async {
      final entries = folders[location.path];
      if (entries == null) throw _failure(FileDialogFailure.notFound);
      return entries;
    },
    resolve: (typed, current) async {
      final path = typed.startsWith('/') ? typed : '${current.path}/$typed';
      final parent = path.substring(0, path.lastIndexOf('/'));
      final target = folders.containsKey(path)
          ? FileDialogPathTarget.folder
          : (folders[parent]?.any((entry) => entry.location.path == path) ??
                false)
          ? FileDialogPathTarget.file
          : folders.containsKey(parent)
          ? FileDialogPathTarget.missing
          : FileDialogPathTarget.unreachable;
      return ResolveFileDialogPathResponse(location: _at(path), target: target);
    },
    createFolder: (parent, name) async {
      created.add(name);
      final folder = _folder(parent.path, name);
      folders[parent.path] = [...folders[parent.path]!, folder];
      folders[folder.location.path] = [];
      return folder.location;
    },
    rename: (location, name) async {
      renamed.add((location.path, name));
      final parent = location.path.substring(0, location.path.lastIndexOf('/'));
      if (folders[parent]!.any((entry) => entry.name == name)) {
        throw _failure(FileDialogFailure.alreadyExists);
      }
      return _at('$parent/$name');
    },
  );
}

class _Harness {
  _Harness(FileDialogOptions options, _FakeFiles files, {bool replace = true})
    : files = files {
    controller = FileDialogController(
      options: options,
      backend: files.backend,
      onAccept: accepted.add,
      onNotice: (notice) async => notices.add(notice),
      confirmReplace: (name) async {
        asked.add(name);
        return replace;
      },
    );
  }

  final _FakeFiles files;
  late final FileDialogController controller;
  final accepted = <FileDialogResult>[];
  final notices = <FileDialogNotice>[];
  final asked = <String>[];

  List<String> get names =>
      controller.visibleEntries.map((entry) => entry.name).toList();
}

_FakeFiles _music() => _FakeFiles({
  '/home': [_folder('/home', 'Music')],
  '/home/Music': [
    _file('/home/Music', 'Track 10.flac'),
    _folder('/home/Music', 'Album 10'),
    _file('/home/Music', 'Track 2.flac'),
    _folder('/home/Music', 'Album 2'),
    _file('/home/Music', 'cover.jpg'),
    _file('/home/Music', '.hidden.flac', hidden: true),
    _folder('/home/Music', '.cache', hidden: true),
  ],
  '/home/Music/Album 2': [_file('/home/Music/Album 2', 'Intro.flac')],
  '/home/Music/Album 10': [],
});

const _audio = [
  FileDialogFileType('Audio', ['flac']),
  FileDialogFileType.all('All Files'),
];

void main() {
  late Directory storage;
  setUpAll(() async {
    TestWidgetsFlutterBinding.ensureInitialized();
    // GetStorage writes its file and a backup from unawaited futures, which can
    // still be on their way when the tests end. Deleting the directory then
    // fails as not empty, or fails the write. Leave the directory for the test
    // process's temporary-directory cleanup.
    storage = await Directory.systemTemp.createTemp('rune-file-dialog-test');
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('plugins.flutter.io/path_provider'),
          (_) async => storage.path,
        );
    StorageKeyManager.initialize(null);
    await SettingsManager().getValue<String>('file-dialog-test');
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

  group('the items shown', () {
    test(
      'put folders first in reading order and keep to the file type',
      () async {
        final harness = _Harness(
          FileDialogOptions(
            mode: FileDialogMode.openFile,
            initialFolder: _at('/home/Music'),
            fileTypes: _audio,
          ),
          _music(),
        );
        await harness.controller.start();
        expect(harness.names, [
          'Album 2',
          'Album 10',
          'Track 2.flac',
          'Track 10.flac',
        ]);

        harness.controller.setFileType(1);
        expect(harness.names, contains('cover.jpg'));

        harness.controller.setShowHidden(true);
        expect(harness.names, containsAll(['.cache', '.hidden.flac']));

        harness.controller.search.text = 'track 1';
        expect(harness.names, ['Track 10.flac']);
      },
    );

    test('are only folders when picking a folder', () async {
      final harness = _Harness(
        FileDialogOptions(
          mode: FileDialogMode.pickFolder,
          initialFolder: _at('/home/Music'),
        ),
        _music(),
      );
      await harness.controller.start();
      expect(harness.names, ['Album 2', 'Album 10']);
    });

    test('follow the chosen sort, folders still first', () async {
      final harness = _Harness(
        FileDialogOptions(
          mode: FileDialogMode.openFile,
          initialFolder: _at('/home/Music'),
          fileTypes: _audio,
        ),
        _music(),
      );
      await harness.controller.start();
      harness.controller.setSort(
        const FileDialogSort(FileDialogColumn.name, false),
      );
      expect(harness.names, [
        'Album 10',
        'Album 2',
        'Track 10.flac',
        'Track 2.flac',
      ]);
    });
  });

  group('navigation', () {
    test(
      'a folder that cannot be read keeps the dialog where it was',
      () async {
        final harness = _Harness(
          FileDialogOptions(
            mode: FileDialogMode.openFile,
            initialFolder: _at('/home/Music'),
          ),
          _music(),
        );
        final controller = harness.controller;
        await controller.start();
        expect(await controller.navigate(_at('/home/Music/Gone')), isFalse);
        expect(controller.current, _at('/home/Music'));
        expect(controller.canGoBack, isFalse);
        expect(harness.notices.single, isA<FileDialogFailureNotice>());
        expect(harness.names, isNotEmpty);
      },
    );

    test('back returns to the folder left with it selected', () async {
      final harness = _Harness(
        FileDialogOptions(
          mode: FileDialogMode.openFile,
          initialFolder: _at('/home/Music'),
        ),
        _music(),
      );
      final controller = harness.controller;
      await controller.start();
      await controller.navigate(_at('/home/Music/Album 2'));
      expect(controller.canGoBack, isTrue);
      await controller.back();
      expect(controller.current, _at('/home/Music'));
      expect(controller.selection, {_at('/home/Music/Album 2')});
      expect(controller.canGoForward, isTrue);
      await controller.forward();
      expect(controller.current, _at('/home/Music/Album 2'));
      await controller.up();
      expect(controller.current, _at('/home/Music'));
      expect(controller.selection, {_at('/home/Music/Album 2')});
    });

    test('a missing last folder falls back to home', () async {
      await SettingsManager().setValue('file_dialog_last_folder', {
        'path': '/home/Gone',
      });
      final harness = _Harness(
        const FileDialogOptions(mode: FileDialogMode.openFile),
        _music(),
      );
      await harness.controller.start();
      expect(harness.controller.current, _at('/home'));
      expect(harness.controller.listingError, isNull);
    });
  });

  group('the file name box', () {
    Future<_Harness> opened(
      FileDialogMode mode, {
      bool replace = true,
      String? suggested,
    }) async {
      final harness = _Harness(
        FileDialogOptions(
          mode: mode,
          initialFolder: _at('/home/Music'),
          fileTypes: _audio,
          suggestedName: suggested,
        ),
        _music(),
        replace: replace,
      );
      await harness.controller.start();
      return harness;
    }

    test(
      'picks a typed file, enters a typed folder, explains a missing one',
      () async {
        final harness = await opened(FileDialogMode.openFile);
        final controller = harness.controller;

        controller.fileName.text = 'Album 2';
        await controller.submit();
        expect(controller.current, _at('/home/Music/Album 2'));
        expect(harness.accepted, isEmpty);

        controller.fileName.text = 'Nope.flac';
        await controller.submit();
        expect(harness.notices.single, isA<FileDialogFileNotFoundNotice>());

        controller.fileName.text = 'Intro.flac';
        await controller.submit();
        expect(harness.accepted.single.locations, [
          _at('/home/Music/Album 2/Intro.flac'),
        ]);
      },
    );

    test('takes several quoted names when opening several files', () async {
      final harness = await opened(FileDialogMode.openFiles);
      harness.controller.fileName.text = '"Track 2.flac" "Track 10.flac"';
      await harness.controller.submit();
      expect(harness.accepted.single.locations, [
        _at('/home/Music/Track 2.flac'),
        _at('/home/Music/Track 10.flac'),
      ]);
    });

    test('turns a wildcard into the filter', () async {
      final harness = await opened(FileDialogMode.openFile);
      harness.controller.setFileType(1);
      harness.controller.fileName.text = '*.jpg';
      await harness.controller.submit();
      expect(harness.names, ['Album 2', 'Album 10', 'cover.jpg']);
      expect(harness.accepted, isEmpty);
    });

    test('follows the selection', () async {
      final harness = await opened(FileDialogMode.openFiles);
      final controller = harness.controller;
      controller.select(_at('/home/Music/Track 2.flac'));
      expect(controller.fileName.text, 'Track 2.flac');
      controller.select(_at('/home/Music/Track 10.flac'), toggle: true);
      expect(controller.fileName.text, '"Track 2.flac" "Track 10.flac"');
      // Shift extends from the last Ctrl-clicked item, Track 10, and
      // replaces the rest of the selection.
      controller.select(_at('/home/Music/Album 10'), range: true);
      expect(controller.selection, {
        _at('/home/Music/Album 10'),
        _at('/home/Music/Track 2.flac'),
        _at('/home/Music/Track 10.flac'),
      });
    });

    test('saving adds the type extension and asks before replacing', () async {
      final declined = await opened(FileDialogMode.saveFile, replace: false);
      declined.controller.fileName.text = 'Track 2';
      await declined.controller.submit();
      expect(declined.asked, ['Track 2.flac']);
      expect(declined.accepted, isEmpty);

      final harness = await opened(FileDialogMode.saveFile);
      harness.controller.fileName.text = 'Track 2';
      await harness.controller.submit();
      expect(harness.accepted.single.location, _at('/home/Music/Track 2.flac'));

      final fresh = await opened(FileDialogMode.saveFile);
      fresh.controller.fileName.text = 'Road Trip';
      await fresh.controller.submit();
      expect(fresh.asked, isEmpty);
      expect(fresh.accepted.single.location, _at('/home/Music/Road Trip.flac'));

      final nowhere = await opened(FileDialogMode.saveFile);
      nowhere.controller.fileName.text = 'Gone/Road Trip';
      await nowhere.controller.submit();
      expect(nowhere.notices.single, isA<FileDialogPathNotFoundNotice>());
    });

    test('changing the save type changes the extension', () async {
      final harness = _Harness(
        const FileDialogOptions(
          mode: FileDialogMode.saveFile,
          suggestedName: 'Road Trip.m3u8',
          initialFolder: FileDialogLocation(
            root: FileDialogRootFileSystem(),
            path: '/home/Music',
          ),
          fileTypes: [
            FileDialogFileType('M3U8', ['m3u8']),
            FileDialogFileType('M3U', ['m3u']),
          ],
        ),
        _music(),
      );
      await harness.controller.start();
      harness.controller.setFileType(1);
      expect(harness.controller.fileName.text, 'Road Trip.m3u');
    });

    test('picks the selected folder, or the shown one', () async {
      final harness = await opened(FileDialogMode.pickFolder);
      await harness.controller.submit();
      expect(harness.accepted.last.location, _at('/home/Music'));
      harness.controller.select(_at('/home/Music/Album 10'));
      harness.controller.fileName.clear();
      await harness.controller.submit();
      expect(harness.accepted.last.location, _at('/home/Music/Album 10'));
    });

    test('the folder box names the folder opened after a selection', () async {
      final harness = await opened(FileDialogMode.pickFolder);
      final controller = harness.controller;
      expect(controller.fileName.text, 'Music');
      controller.select(_at('/home/Music/Album 2'));
      expect(controller.fileName.text, 'Album 2');
      // Opening the selected folder: the box names it, and picking takes
      // it rather than an "Album 2" inside it.
      await controller.navigate(_at('/home/Music/Album 2'));
      expect(controller.fileName.text, 'Album 2');
      await controller.submit();
      expect(harness.notices, isEmpty);
      expect(controller.fileName.text, 'Album 2');
      expect(harness.accepted.last.location, _at('/home/Music/Album 2'));
      // Back out with nothing selected: the box names the folder shown.
      await controller.navigate(_at('/home'));
      controller.clearSelection();
      expect(controller.fileName.text, 'home');
      // A typed name is still looked up where the dialog is.
      controller.fileName.text = 'Nowhere';
      await controller.submit();
      expect(harness.notices.single, isA<FileDialogFolderNotFoundNotice>());
    });
  });

  group('preferences', () {
    Future<FileDialogController> reopened() async {
      final harness = _Harness(
        FileDialogOptions(
          mode: FileDialogMode.openFile,
          initialFolder: _at('/home/Music'),
        ),
        _music(),
      );
      await harness.controller.start();
      return harness.controller;
    }

    test('the view, columns and pane share survive the dialog', () async {
      final first = await reopened();
      expect(first.view, FileDialogView.details);
      first.setView(FileDialogView.icons(150));
      first.setColumnWidth(FileDialogColumn.name, 300);
      first.setColumnShown(FileDialogColumn.type, false);
      // The name cannot be switched off.
      first.setColumnShown(FileDialogColumn.name, false);
      first.setNavigationShare(0.3, save: true);
      // Closing writes what a burst of changes left pending.
      first.dispose();

      final second = await reopened();
      expect(second.view, FileDialogView.icons(150));
      expect(second.columnWidths, {FileDialogColumn.name: 300});
      expect(second.hiddenColumns, {FileDialogColumn.type});
      expect(second.navigationShare, 0.3);
      second.setColumnWidth(FileDialogColumn.name, null);
      second.setColumnShown(FileDialogColumn.type, true);
      second.dispose();

      final third = await reopened();
      expect(third.columnWidths, isEmpty);
      expect(third.hiddenColumns, isEmpty);
    });
  });

  group('changes', () {
    test('a new folder gets a free name and is renamed in place', () async {
      final files = _music();
      files.folders['/home/Music']!.add(_folder('/home/Music', 'New Folder'));
      final harness = _Harness(
        FileDialogOptions(
          mode: FileDialogMode.openFile,
          initialFolder: _at('/home/Music'),
        ),
        files,
      );
      final controller = harness.controller;
      await controller.start();
      await controller.createFolder('New Folder');
      expect(files.created, ['New Folder (2)']);
      expect(controller.renaming, _at('/home/Music/New Folder (2)'));
      expect(controller.selection, {_at('/home/Music/New Folder (2)')});
    });

    test('a refused rename keeps the editor open and explains why', () async {
      final harness = _Harness(
        FileDialogOptions(
          mode: FileDialogMode.openFile,
          initialFolder: _at('/home/Music'),
        ),
        _music(),
      );
      final controller = harness.controller;
      await controller.start();
      controller.select(_at('/home/Music/Album 2'));
      controller.startRename();
      expect(await controller.commitRename('Album 10'), isFalse);
      expect(controller.renaming, _at('/home/Music/Album 2'));
      expect(harness.notices.single, isA<FileDialogFailureNotice>());
      expect(await controller.commitRename('Album 2'), isTrue);
      expect(controller.renaming, isNull);
      expect(harness.files.renamed, [('/home/Music/Album 2', 'Album 10')]);
    });
  });

  group('the model', () {
    test('parents and names follow each platform\'s paths', () {
      expect(parentOf(_at('/home/Music'))?.path, '/home');
      expect(parentOf(_at('/home'))?.path, '/');
      expect(parentOf(_at('/')), isNull);
      expect(parentOf(_at(r'C:\Music'))?.path, r'C:\');
      expect(parentOf(_at(r'C:\')), isNull);
      expect(nameOf(_at('/home/Music/')), 'Music');
      expect(nameOf(_at('/')), '/');
      const tree = FileDialogRootDocumentTree(uri: 'content://tree');
      expect(
        parentOf(const FileDialogLocation(root: tree, path: 'Music')),
        const FileDialogLocation(root: tree, path: ''),
      );
      expect(parentOf(const FileDialogLocation(root: tree, path: '')), isNull);
      expect(ancestry(_at('/home/Music')).map((location) => location.path), [
        '/',
        '/home',
        '/home/Music',
      ]);
    });

    test('names compare as people read them', () {
      final names = ['Track 10', 'track 2', 'Track 1', 'Album'];
      names.sort(compareNatural);
      expect(names, ['Album', 'Track 1', 'track 2', 'Track 10']);
    });

    test('sizes read like File Explorer', () {
      expect(detailsSize(1), '1 KB');
      expect(detailsSize(1024 * 1500), '1,500 KB');
      expect(shortSize(512), '512 bytes');
      expect(shortSize(3 * 1024 * 1024 + 300 * 1024), '3.29 MB');
      expect(extensionOf('.profile'), '');
      expect(extensionOf('Mix.M3U8'), 'm3u8');
      expect(stemOf('Mix.m3u8'), 'Mix');
    });
  });
}
