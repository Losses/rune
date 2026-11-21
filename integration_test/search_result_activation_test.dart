import 'dart:convert';
import 'dart:io';

import 'package:bitsdojo_window/bitsdojo_window.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:path_provider_platform_interface/path_provider_platform_interface.dart';
import 'package:provider/provider.dart';
import 'package:rinf/rinf.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/main.dart' as app;
import 'package:rune/providers/library_manager.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/providers/router_path.dart';
import 'package:rune/screens/home/widgets/welcome.dart';
import 'package:rune/screens/search/search.dart';
import 'package:rune/screens/search/widgets/collection_search_item.dart';
import 'package:rune/screens/search/widgets/track_search_item.dart';
import 'package:rune/utils/router/navigation.dart';
import 'package:rune/utils/router/query_tracks_parameter.dart';
import 'package:rune/utils/router/router_transition_parameter.dart';

import 'runtime_probe_support.dart';

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  binding.framePolicy = LiveTestWidgetsFlutterBindingFramePolicy.benchmarkLive;

  testWidgets('Zune search results queue the selected track and open its album', (
    tester,
  ) async {
    final root = await Directory.systemTemp.createTemp('rune-search-');
    addTearDown(() async {
      try {
        await root.delete(recursive: true);
      } on FileSystemException catch (error) {
        // Windows keeps GetStorage and SQLite handles open until the process exits.
        if (!Platform.isWindows || error.osError?.errorCode != 32) rethrow;
        debugPrint('RUNE_SEARCH_TEST_STORAGE=${root.path}');
      }
    });
    final music = await Directory('${root.path}/music').create();
    final config = await Directory('${root.path}/config').create();
    final paths = PathProviderPlatform.instance;
    PathProviderPlatform.instance = _Paths(config.path);
    final dispatcher = binding.platformDispatcher;
    final onError = dispatcher.onError;
    final onBrightnessChanged = dispatcher.onPlatformBrightnessChanged;
    addTearDown(() {
      PathProviderPlatform.instance = paths;
      dispatcher.onError = onError;
      dispatcher.onPlatformBrightnessChanged = onBrightnessChanged;
    });

    const title = 'Searchable Track';
    const album = 'Searchable Album';
    // One second of silent FLAC, tagged with the title and album above.
    await File('${music.path}/track.flac').writeAsBytes(
      base64Decode(
        'ZkxhQwAAACIfQB9AAAANAAANAfQA8AAAH0Ae4Bk2cWCcfWPP6JuSCtMThAAAWgYAAABm'
        'Zm1wZWcDAAAAFgAAAHRpdGxlPVNlYXJjaGFibGUgVHJhY2sWAAAAYWxidW09U2VhcmNo'
        'YWJsZSBBbGJ1bRQAAABhcnRpc3Q9U2VhcmNoIEFydGlzdP/4dAgAHz+8AAAAefg=',
      ),
    );

    try {
      app.main(const []);
      final welcome = find.byType(WelcomePage);
      await probeWait(tester, () => welcome.evaluate().isNotEmpty, 'Welcome');
      final context = tester.element(welcome);
      final manager = context.read<LibraryManagerProvider>();
      final responsive = context.read<ResponsiveProvider>();
      final record = (await app.libraryRegistry.registerLocalFolder(
        music.path,
        'Search test',
        LibraryInitializeMode.portable,
      )).record!;
      await app.libraryRegistry.open(record.recordId);
      await manager.scanLibrary(record.recordId);
      expect(
        await manager.waitForScanToComplete(record.recordId),
        TaskStatus.finished,
      );

      $$replace('/library');
      await probeWait(
        tester,
        () => $hasLibraryNavigator,
        'the library navigator',
      );
      appWindow.size = const Size(340, 700);
      await probeWait(
        tester,
        () => responsive.currentDeviceType == DeviceType.zune,
        'the Zune layout',
      );
      debugPrint(
        'Search window: ${appWindow.size}, layout: ${responsive.currentDeviceType}',
      );
      $push('/search');
      await probeWait(
        tester,
        () => find.byType(SearchPage).evaluate().isNotEmpty,
        'the search page',
      );
      await tester.enterText(find.byType(EditableText), 'Searchable');

      final trackResult = find.widgetWithText(TrackSearchItem, title);
      final albumResult = find.widgetWithText(CollectionSearchItem, album);
      await probeWait(
        tester,
        () =>
            trackResult.hitTestable().evaluate().isNotEmpty &&
            albumResult.hitTestable().evaluate().isNotEmpty,
        'the album and track results',
      );
      final track = tester.widget<TrackSearchItem>(trackResult).item;
      final collection = tester.widget<CollectionSearchItem>(albumResult).item;
      debugPrint(
        'Search results: track ${track.id} ($title), album ${collection.id} ($album)',
      );
      final expectedTrack = PlayingItemRefLibraryTrack(
        libraryId: track.libraryId,
        track: track.track,
      );
      expect(PlaylistUpdate.latestRustSignal?.message.items ?? [], isEmpty);
      await tester.tap(trackResult);
      await probeWait(
        tester,
        () =>
            PlaylistUpdate.latestRustSignal?.message.items.singleOrNull?.item ==
            expectedTrack,
        'the selected track to enter the playback queue',
      );
      debugPrint(
        'Queued the selected track: ${track.libraryId}/${track.track}',
      );
      await tester.tap(albumResult);
      await probeWait(
        tester,
        () => $router.path == '/albums/detail',
        'the album detail page',
      );
      final route = $router.parameter as RouterTransitionParameter;
      final selected = route.parameters as QueryTracksParameter;
      expect(selected.id, collection.id);
      expect(selected.title, album);
      debugPrint('Opened album ${selected.id} (${selected.title})');
    } finally {
      await tester.pumpWidget(const SizedBox.shrink());
      finalizeRust();
    }
  }, timeout: const Timeout(Duration(minutes: 4)));
}

class _Paths extends PathProviderPlatform {
  _Paths(this.path);
  final String path;

  @override
  Future<String> getApplicationDocumentsPath() async => path;
  @override
  Future<String> getApplicationSupportPath() async => path;
  @override
  Future<String> getTemporaryPath() async => path;
}
