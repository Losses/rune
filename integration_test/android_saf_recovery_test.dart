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
    final library = await openGrantedFixture(
      tester,
      'primary:Music/$name',
      prepareFixture: () async {
        expect(await root.exists(), isFalse);
        expect(await moved.exists(), isFalse);
        await root.create(recursive: true);
        addTearDown(() async {
          for (final dir in [root, moved]) {
            if (await dir.exists()) await dir.delete(recursive: true);
          }
        });
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
    await tapWhenReady(
      tester,
      find.descendant(
        of: find.byType(ScanLibraryButton),
        matching: find.byType(Button),
      ),
    );
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
    final pickDirectory = picker.pickDirectory;
    final selection = Completer<String?>();
    picker.pickDirectory = (context) {
      picker.pickDirectory = pickDirectory;
      final result = pickDirectory(context);
      selection.complete(result);
      return result;
    };
    await tapWhenReady(tester, choose);
    if (!selection.isCompleted) await tester.pump();
    // DocumentsUI owns the window until the host confirms this selection.
    expect(await selection.future.timeout(const Duration(seconds: 60)), uri);
    await lifecycle.waitFor(
      tester,
      () =>
          manager.getScanTaskProgress(recordId)?.status == TaskStatus.finished,
      description: 'the scan after regrant to finish',
    );
    expect(await saf.hasPersistedPermission(uri), isTrue);
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
