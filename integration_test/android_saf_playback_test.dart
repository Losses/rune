import 'dart:async';
import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:permission_handler/permission_handler.dart';
import 'package:provider/provider.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/main.dart' as app;
import 'package:rune/providers/library_manager.dart';
import 'package:rune/providers/library_open_state.dart';
import 'package:rune/providers/local_folder_picker.dart';
import 'package:rune/screens/home/widgets/welcome.dart';
import 'package:rune/screens/library_home/library_home.dart';
import 'package:rune/screens/tracks/tracks.dart';
import 'package:rune/test/android_playback_progress_test.dart'
    show trackProgress;
import 'package:rune/utils/api/fetch_media_files.dart';
import 'package:rune/utils/api/get_media_files_count.dart';
import 'package:rune/utils/l10n.dart';
import 'package:saf_util/saf_util.dart';

import 'e2e_library_flow_test.dart' as lifecycle;

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('play a scanned track from the granted Android SAF tree', (
    tester,
  ) async {
    const treeId = String.fromEnvironment('RUNE_SAF_TREE_ID');
    const filename = 'startup_0.ogg';
    const authority = 'com.android.externalstorage.documents';
    expect(Platform.isAndroid, isTrue);
    expect(treeId, isNotEmpty);

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

    await _tap(tester, find.text(labels.addLibrary));
    await _tap(tester, find.text(labels.localFolder));
    await _tap(tester, find.widgetWithText(Button, labels.confirm));
    await _tap(tester, find.text(labels.localMode));
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

    final count = await getMediaFilesCount();
    expect(count, greaterThan(0), reason: 'the scan ingested no tracks');
    final files = await fetchMediaFiles(0, count);
    // The host places this untagged track in a fresh tree. Rune displays its
    // filename as the title even when it cannot resolve a playable path.
    final matches = files.where((file) => file.title == filename);
    expect(
      matches,
      hasLength(1),
      reason:
          'expected $filename in the scanned catalog, got '
          '${files.map((file) => file.title).toList()}',
    );
    final fixture = matches.single;
    expect(fixture.libraryId, isNotEmpty);
    expect(fixture.libraryId, library.catalogLibraryId);
    expect(fixture.track, isNotEmpty);
    final expected = PlayingItemRefLibraryTrack(
      libraryId: fixture.libraryId,
      track: fixture.track,
    );
    debugPrint(
      'Scanned $filename library=${fixture.libraryId} track=${fixture.track} '
      'availability=${fixture.audioAvailability.name} path="${fixture.path}"',
    );
    await _tap(tester, find.text(labels.tracks));
    final rowTitle = find.descendant(
      of: find.byType(TracksPage),
      matching: find.text(fixture.title),
    );
    expect(
      PlaybackStatus.latestRustSignal?.message.state,
      isNot(PlaybackState.playing),
    );
    await _tap(tester, rowTitle);
    final (first, last) = await trackProgress(
      PlaybackStatus.rustSignalStream.map((signal) => signal.message),
      expected,
    );
    debugPrint(
      'Verified Android SAF playback: $filename '
      'library=${fixture.libraryId} track=${fixture.track} '
      'progress=${first}s -> ${last}s scanned=$count',
    );
  }, timeout: const Timeout(Duration(minutes: 4)));
}

Future<void> _tap(WidgetTester tester, Finder matches) async {
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
