import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/screens/tracks/tracks.dart';
import 'package:rune/test/android_playback_progress_test.dart'
    show trackProgress;
import 'package:rune/utils/api/fetch_media_files.dart';
import 'package:rune/utils/api/get_media_files_count.dart';

import 'android_saf_test_support.dart';
import 'e2e_library_flow_test.dart' as lifecycle;

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('play a scanned track from the granted Android SAF tree', (
    tester,
  ) async {
    const treeId = String.fromEnvironment('RUNE_SAF_TREE_ID');
    const filename = 'startup_0.ogg';
    expect(Platform.isAndroid, isTrue);
    expect(treeId, isNotEmpty);

    final library = await openGrantedFixture(tester, treeId);
    final labels = lifecycle.localized(tester);
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
    await tapWhenReady(tester, find.text(labels.tracks));
    final rowTitle = find.descendant(
      of: find.byType(TracksPage),
      matching: find.text(fixture.title),
    );
    expect(
      PlaybackStatus.latestRustSignal?.message.state,
      isNot(PlaybackState.playing),
    );
    await tapWhenReady(tester, rowTitle);
    final (first, last) = await trackProgress(
      PlaybackStatus.rustSignalStream.map((signal) => signal.message),
      expected,
    );
    expect(fixture.audioAvailability, AudioAvailability.local);
    expect(fixture.audioOrigin, AudioOrigin.host);
    debugPrint(
      'Verified Android SAF playback: $filename '
      'library=${fixture.libraryId} track=${fixture.track} '
      'progress=${first}s -> ${last}s scanned=$count',
    );
  }, timeout: const Timeout(Duration(minutes: 4)));
}
