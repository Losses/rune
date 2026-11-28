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

const _treeId = String.fromEnvironment('RUNE_SAF_TREE_ID');
const _authority = 'com.android.externalstorage.documents';
// Run seed with the earlier APK and --no-uninstall. Then force-stop the
// package and pass the emitted identity to reopen with the replacement APK.
const _phase = String.fromEnvironment('RUNE_SAF_PHASE', defaultValue: 'fresh');

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('Android SAF playback ($_phase)', (tester) async {
    expect(Platform.isAndroid, isTrue);
    expect(_treeId, isNotEmpty);
    expect(['fresh', 'seed', 'reopen'], contains(_phase));
    app.main(const []);
    if (_phase != 'reopen') await _register(tester);

    final home = find.byType(LibraryHomePage);
    await lifecycle.expectVisible(tester, home);
    final context = tester.element(home);
    final library = context.read<LibraryOpenStateProvider>();
    final granted = Uri.parse(library.openLocalFolderPath!);
    expect(granted.scheme, 'content');
    expect(granted.authority, _authority);
    expect(granted.pathSegments, ['tree', _treeId, 'document', _treeId]);
    expect(await SafUtil().hasPersistedPermission(granted.toString()), isTrue);

    final count = await getMediaFilesCount();
    expect(count, greaterThan(0), reason: 'the scan ingested no tracks');
    final files = await fetchMediaFiles(0, count);
    final names = ['startup_0.ogg', if (_phase != 'fresh') 'startup_1.ogg'];
    final fixtures = names.map((name) {
      final matches = files.where((file) => file.title == name);
      expect(matches, hasLength(1), reason: '$name in the SAF catalog');
      final fixture = matches.single;
      expect(fixture.libraryId, isNotEmpty);
      expect(fixture.libraryId, library.catalogLibraryId);
      expect(fixture.track, isNotEmpty);
      debugPrint(
        'Scanned $name library=${fixture.libraryId} track=${fixture.track} '
        'availability=${fixture.audioAvailability.name}',
      );
      return fixture;
    }).toList();

    // The host carries the scan's IDs across APK installs, so reopening
    // an empty or newly registered library cannot satisfy the playback test.
    final identity = [
      library.openRecord!.recordId,
      library.catalogLibraryId,
      for (final fixture in fixtures) fixture.track,
    ].join(',');
    if (_phase == 'seed') {
      debugPrint('Seeded Android SAF library: $identity');
      return;
    }
    if (_phase == 'reopen') {
      const seed = String.fromEnvironment('RUNE_SAF_SEED_IDENTITY');
      expect(identity, seed, reason: 'the saved library and track IDs');
    }

    expect(
      PlaybackStatus.latestRustSignal?.message.state,
      isNot(PlaybackState.playing),
    );
    await _tap(tester, find.text(S.of(context).tracks));
    for (final fixture in fixtures) {
      final expected = PlayingItemRefLibraryTrack(
        libraryId: fixture.libraryId,
        track: fixture.track,
      );
      await _tap(
        tester,
        find.descendant(
          of: find.byType(TracksPage),
          matching: find.text(fixture.title),
        ),
      );
      final (first, last) = await trackProgress(
        PlaybackStatus.rustSignalStream.map((signal) => signal.message),
        expected,
      );
      expect(fixture.audioAvailability, AudioAvailability.local);
      expect(fixture.audioOrigin, AudioOrigin.host);
        expect(
          fixture.path,
          fixture.title == 'startup_1.ogg'
              ? 'Album/startup_1.ogg'
              : 'startup_0.ogg',
          reason: 'the catalog path is relative to the granted tree',
        );
      expect(
        PlaylistUpdate.latestRustSignal?.message.items.map((row) => row.item),
        contains(expected),
        reason: 'playing a saved track keeps it in the queue',
      );
      debugPrint(
        'Verified Android SAF playback: ${fixture.title} '
        'library=${fixture.libraryId} track=${fixture.track} '
        'progress=${first}s -> ${last}s scanned=$count',
      );
    }
  }, timeout: const Timeout(Duration(minutes: 4)));
}

Future<void> _register(WidgetTester tester) async {
  final welcome = find.byType(WelcomePage);
  await lifecycle.expectVisible(tester, welcome);
  final context = tester.element(welcome);
  final labels = S.of(context);
  final library = context.read<LibraryOpenStateProvider>();
  final manager = context.read<LibraryManagerProvider>();
  // Start the real picker at this run's fixture. The host confirms it.
  context.read<LocalFolderPickerProvider>().pickDirectory = (_) async {
    final picked = await SafUtil().pickDirectory(
      initialUri: Uri(
        scheme: 'content',
        host: _authority,
        pathSegments: ['document', _treeId],
      ).toString(),
      persistablePermission: true,
      writePermission: true,
    );
    return picked?.uri;
  };
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
  expect(
    await manager.waitForScanToComplete(library.openRecord!.recordId),
    TaskStatus.finished,
    reason: 'the initial scan must finish for the selected SAF library',
  );
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
