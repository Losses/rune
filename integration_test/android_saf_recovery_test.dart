import 'dart:async';
import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/providers/library_manager.dart';
import 'package:rune/providers/local_folder_picker.dart';
import 'package:rune/screens/settings_library/settings_library.dart';
import 'package:rune/utils/api/create_m3u8_playlist.dart';
import 'package:rune/utils/api/fetch_media_files.dart';
import 'package:rune/utils/api/get_media_files_count.dart';
import 'package:rune/utils/api/request_failure.dart';
import 'package:rune/utils/l10n.dart';
import 'package:rune/utils/library_ui/library_ui_state.dart';
import 'package:rune/utils/router/navigation.dart';
import 'package:rune/widgets/library_task_button.dart';
import 'package:rune/widgets/track_list/utils/internal_media_file.dart';
import 'package:saf_util/saf_util.dart';

import 'android_saf_test_support.dart';
import 'e2e_library_flow_test.dart' as lifecycle;

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('regrant the library folder and resume scanning its catalog', (
    tester,
  ) async {
    const name = String.fromEnvironment('RUNE_SAF_FIXTURE_NAME');
    expect(Platform.isAndroid, isTrue);
    expect(name, matches(RegExp(r'^rune-saf-recovery-[a-f0-9]{32}$')));
    final root = Directory('/sdcard/Music/$name');
    final moved = Directory('${root.path}.moved');
    final existing = Directory('${root.path}-existing');
    final ungranted = Directory('${root.path}-new');
    final library = await openGrantedFixture(
      tester,
      'primary:Music/$name',
      prepareFixture: () async {
        expect(await root.exists(), isFalse);
        expect(await moved.exists(), isFalse);
        for (final dir in [existing, ungranted]) {
          expect(await dir.exists(), isFalse);
        }
        addTearDown(() async {
          for (final dir in [root, moved, existing, ungranted]) {
            if (await dir.exists()) await dir.delete(recursive: true);
          }
        });
        for (final dir in [root, existing, ungranted]) {
          await dir.create(recursive: true);
        }
        await _writeTrack(root, 'startup_0.ogg');
      },
    );
    final uri = library.openLocalFolderPath!;
    final recordId = library.openRecord!.recordId;
    final process = pid;
    final before = await _catalog();
    expect(before.keys, ['startup_0.ogg']);
    final original = before['startup_0.ogg']!;
    expect(original.libraryId, library.catalogLibraryId);
    expect(original.track, isNotEmpty);

    final saf = SafUtil();
    // Start with read-only access to another folder and no grant for a third.
    final existingGrant = await saf.pickDirectory(
      initialUri: _fixtureUri('$name-existing').toString(),
      persistablePermission: true,
      writePermission: false,
    );
    final existingUri = existingGrant!.uri;
    final newUri = _fixtureUri('$name-new', withinTree: true).toString();
    final expectedExisting = _fixtureUri('$name-existing', withinTree: true);
    expect(Uri.parse(existingUri).authority, expectedExisting.authority);
    expect(Uri.parse(existingUri).pathSegments, expectedExisting.pathSegments);
    expect(await _persistedAccess(saf, existingUri), (true, false));
    expect(await _persistedAccess(saf, newUri), isNull);
    final playlistFile = File('${root.path}/fixture.m3u8');
    const playlist = '#EXTM3U\nstartup_0.ogg\n';
    await playlistFile.writeAsString(playlist);
    final document = await saf.child(uri, ['fixture.m3u8']);
    expect(document, isNotNull);
    final playlistUri = document!.uri;
    Future<CreateM3u8PlaylistResponse> importPlaylist() =>
        createM3u8Playlist(name, 'Tests', playlistUri);
    final unavailable = throwsA(
      isA<RequestFailure>().having(
        (e) => e.kind,
        'kind',
        RequestErrorKind.unavailable,
      ),
    );
    // Open the same document after deletion, after grant loss, and after regrant.
    await playlistFile.delete();
    await expectLater(importPlaylist(), unavailable);
    await playlistFile.writeAsString(playlist);

    // ExternalStorageProvider changes the document ID on rename. Android then
    // revokes the old tree's grants, including temporary and persisted access.
    await saf.rename(uri, true, '$name.moved');
    expect(await moved.exists(), isTrue);
    expect(await saf.hasPersistedPermission(uri), isFalse);
    // Restore the same files before scanning so the missing grant alone keeps
    // Rune from seeing the new track. Raw paths are only used to stage fixtures.
    await moved.rename(root.path);
    await _writeTrack(root, 'startup_1.ogg');
    await expectLater(importPlaylist(), unavailable);

    $push('/settings/library');
    final page = find.byType(SettingsLibraryPage);
    await lifecycle.expectVisible(tester, page);
    final context = tester.element(page);
    final manager = context.read<LibraryManagerProvider>();
    final labels = S.of(context);
    await tapWhenReady(tester, find.text(localFolderName(uri)));
    final scanButton = find.descendant(
      of: find.byType(ScanLibraryButton),
      matching: find.byType(Button),
    );
    await tapWhenReady(tester, scanButton);
    final choose = find.text(labels.pickDirectory);
    await lifecycle.expectVisible(tester, choose);
    expect(manager.getScanTaskProgress(recordId)?.status, TaskStatus.failed);
    expect(
      manager.getScanTaskProgress(recordId)?.scanFailure,
      ScanFailure.documentFolderUnavailable,
    );
    // Both the database count and the public track listing survive grant loss.
    expect(await getMediaFilesCount(), before.length);
    final denied = (await _catalog())[original.title]!;
    expect(_identity(denied), _identity(original));
    expect(denied.audioAvailability, isNot(AudioAvailability.local));
    expect(denied.path, isEmpty);
    expect(pid, process);

    final picker = context.read<LocalFolderPickerProvider>();
    final pickRecoveryFolder = picker.pickRecoveryFolder;
    Future<String> chooseFolder(String targetName) async {
      final selection = Completer<RecoveryFolder?>();
      picker.pickRecoveryFolder = (context, currentPath) {
        picker.pickRecoveryFolder = pickRecoveryFolder;
        expect(currentPath, uri);
        // Only the starting location changes. Use the production picker and
        // its actual Android result flags for both accepted and rejected trees.
        final result = pickRecoveryFolder(
          context,
          _fixtureUri(targetName).toString(),
        );
        selection.complete(result);
        return result;
      };
      await tapWhenReady(tester, choose);
      if (!selection.isCompleted) await tester.pump();
      final result = await selection.future.timeout(
        const Duration(seconds: 60),
      );
      final tree = Uri.parse(result!.uri);
      expect(tree.authority, _fixtureUri(targetName).authority);
      expect(tree.pathSegments, ['tree', 'primary:Music/$targetName']);
      // saf_util takes a root document URI and matches its document ID against
      // Android's persisted tree IDs. Derive it from the actual picker result.
      return tree
          .replace(
            pathSegments: [
              ...tree.pathSegments,
              'document',
              tree.pathSegments.last,
            ],
          )
          .toString();
    }

    for (final kind in ['new', 'existing']) {
      final selectedUri = await chooseFolder('$name-$kind');
      await lifecycle.expectVisible(
        tester,
        find.text(labels.selectOriginalLibraryFolder),
      );
      final newAccess = await _persistedAccess(
        saf,
        kind == 'new' ? selectedUri : newUri,
      );
      final existingAccess = await _persistedAccess(
        saf,
        kind == 'existing' ? selectedUri : existingUri,
      );
      final originalAccess = await _persistedAccess(saf, uri);
      expect(newAccess, isNull);
      expect(existingAccess, (true, false));
      expect(originalAccess, isNull);
      debugPrint(
        'Verified rejected SAF folder ($kind), persisted (read, write): '
        'new=$newAccess existing=$existingAccess original=$originalAccess',
      );
      expect(manager.getScanTaskProgress(recordId)?.status, TaskStatus.failed);
      await tapWhenReady(tester, find.text(labels.close));
      await tapWhenReady(tester, scanButton);
      await lifecycle.expectVisible(tester, choose);
    }
    final restoredUri = await chooseFolder(name);
    await lifecycle.waitFor(
      tester,
      () =>
          manager.getScanTaskProgress(recordId)?.status == TaskStatus.finished,
      description: 'the scan after regrant to finish',
    );
    expect(await _persistedAccess(saf, restoredUri), (true, true));
    expect(library.openRecord?.recordId, recordId);
    final after = await _catalog();
    expect(after.keys, unorderedEquals(['startup_0.ogg', 'startup_1.ogg']));
    final restored = after[original.title]!;
    expect(_identity(restored), _identity(original));
    expect(restored.audioAvailability, original.audioAvailability);
    expect(restored.path, original.path);
    final added = after['startup_1.ogg']!;
    expect(added.libraryId, original.libraryId);
    expect(added.track, isNotEmpty);
    expect(added.track, isNot(original.track));
    expect((await importPlaylist()).importedCount, 1);
    expect(pid, process);
    debugPrint(
      'Verified Android SAF recovery: pid=$process record=$recordId '
      'retained=${original.track} added=${added.track}',
    );
  }, timeout: const Timeout(Duration(minutes: 4)));
}

Future<(bool, bool)?> _persistedAccess(SafUtil saf, String uri) async {
  // With both flags disabled, saf_util reports any persisted entry, including
  // one with only write access. Read and write are then queried separately.
  if (!await saf.hasPersistedPermission(
    uri,
    checkRead: false,
    checkWrite: false,
  )) {
    return null;
  }
  return (
    await saf.hasPersistedPermission(uri),
    await saf.hasPersistedPermission(uri, checkRead: false, checkWrite: true),
  );
}

Uri _fixtureUri(String name, {bool withinTree = false}) {
  final id = 'primary:Music/$name';
  return Uri(
    scheme: 'content',
    host: 'com.android.externalstorage.documents',
    pathSegments: [
      if (withinTree) ...['tree', id],
      'document',
      id,
    ],
  );
}

Future<void> _writeTrack(Directory root, String name) async {
  final bytes = await rootBundle.load('assets/$name');
  await File('${root.path}/$name').writeAsBytes(
    bytes.buffer.asUint8List(bytes.offsetInBytes, bytes.lengthInBytes),
  );
}

(String, String, int) _identity(InternalMediaFile file) =>
    (file.libraryId, file.track, file.id);

Future<Map<String, InternalMediaFile>> _catalog() async {
  final count = await getMediaFilesCount();
  final files = await fetchMediaFiles(0, count);
  final tracks = {for (final file in files) file.title: file};
  expect(tracks.length, count);
  return tracks;
}
