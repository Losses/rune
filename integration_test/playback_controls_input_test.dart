import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' show PlatformDispatcher;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:path_provider_platform_interface/path_provider_platform_interface.dart';
import 'package:rinf/rinf.dart';

import 'package:rune/bindings/bindings.dart';
import 'package:rune/main.dart' as app;
import 'package:rune/screens/home/widgets/welcome.dart';
import 'package:rune/screens/library_home/library_home.dart';
import 'package:rune/utils/api/request_failure.dart';
import 'package:rune/utils/router/navigation.dart';
import 'package:rune/widgets/banding_animation/branding_animation.dart';
import 'package:rune/widgets/playback_controller/next_button.dart';
import 'package:rune/widgets/playback_controller/play_pause_button.dart';
import 'package:rune/widgets/playback_controller/playback_controller.dart';
import 'package:rune/widgets/playback_controller/previous_button.dart';

// The real library page and the playback bar share a hit test in the router.
// Exercise both together against a player, settings and library owned by the test.
void main() {
  // Let the desktop engine drive frames while the checks wait for actual
  // widget and player state, including when the window is in the background.
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  binding.framePolicy = LiveTestWidgetsFlutterBindingFramePolicy.benchmarkLive;
  // The test runner can draw before Rune finishes resizing its macOS window.
  // Match a normal launch by submitting the first frame at the final size.
  if (Platform.isMacOS) binding.deferFirstFrame();
  testWidgets('only primary clicks pause or skip tracks on the library page', (
    tester,
  ) async {
    final directory = await Directory.systemTemp.createTemp('rune-input-');
    final config = await Directory('${directory.path}/config').create();
    final paths = PathProviderPlatform.instance;
    PathProviderPlatform.instance = _TestPaths(config.path);
    final dispatcher = PlatformDispatcher.instance;
    final onError = dispatcher.onError;
    final onBrightnessChanged = dispatcher.onPlatformBrightnessChanged;
    addTearDown(() {
      PathProviderPlatform.instance = paths;
      dispatcher.onError = onError;
      dispatcher.onPlatformBrightnessChanged = onBrightnessChanged;
    });

    PlaybackStatus? status;
    final statuses = PlaybackStatus.rustSignalStream.listen(
      (event) => status = event.message,
    );
    const transportRequests = {
      'PlayRequest',
      'PauseRequest',
      'NextRequest',
      'PreviousRequest',
    };
    final failures = <RequestFailedResponse>[];
    final errors = RequestFailedResponse.rustSignalStream
        .where((event) => transportRequests.contains(event.message.request))
        .listen((event) => failures.add(event.message));
    try {
      app.main(const []);
      try {
        await _waitFor(
          tester,
          () => find.byType(WelcomePage).evaluate().isNotEmpty,
          'Rune to open with an empty library registry',
        );
      } finally {
        if (Platform.isMacOS) binding.allowFirstFrame();
      }
      final library = await Directory('${directory.path}/library').create();
      final registered = await app.libraryRegistry.registerLocalFolder(
        library.path,
        'Input test',
        LibraryInitializeMode.portable,
      );
      await app.libraryRegistry.open(registered.record!.recordId);
      $$replace('/library');
      const SetAdaptiveSwitchingEnabledRequest(
        enabled: false,
      ).sendSignalToRust();
      final bar = find.byType(PlaybackController);
      await _waitFor(
        tester,
        () =>
            find.byType(LibraryHomePage).evaluate().isNotEmpty &&
            bar.evaluate().length == 1 &&
            find.byType(BrandingAnimation).evaluate().isEmpty,
        'the library home page and its playback bar',
      );
      final items = await _silentTracks(library);
      final controls = {
        'play/pause': find.descendant(
          of: bar,
          matching: find.byType(PlayPauseButton),
        ),
        'next': find.descendant(of: bar, matching: find.byType(NextButton)),
        'previous': find.descendant(
          of: bar,
          matching: find.byType(PreviousButton),
        ),
      };
      for (final control in controls.entries) {
        debugPrint('Checking secondary and primary ${control.key}');
        await _load(items);
        await _waitFor(
          tester,
          () =>
              controls.values.every(
                (control) => control.evaluate().length == 1,
              ) &&
              status?.ready == true &&
              status?.state == PlaybackState.playing &&
              status?.itemRequest == items[1] &&
              tester.widget<PlayPauseButton>(controls['play/pause']!).state ==
                  PlaybackState.playing &&
              !tester
                  .widget<PlayPauseButton>(controls['play/pause']!)
                  .disabled &&
              !tester.widget<PreviousButton>(controls['previous']!).disabled &&
              !tester.widget<NextButton>(controls['next']!).disabled,
          'the middle track and enabled controls',
        );
        final before = status!.progressSeconds;
        await _mouseClick(tester, control.value, kSecondaryMouseButton);
        // Observe actual playback progress while requiring unchanged transport
        // state; native request types have independent receivers, so another
        // request's response would not be an ordering barrier.
        await _waitFor(tester, () {
          expect(
            status!.state,
            PlaybackState.playing,
            reason: 'secondary ${control.key} changed playback state',
          );
          expect(
            status!.itemRequest,
            items[1],
            reason: 'secondary ${control.key} changed the track',
          );
          expect(failures, isEmpty);
          return status!.progressSeconds > before + 0.5;
        }, 'playback to continue after secondary ${control.key}');
        await _mouseClick(tester, control.value, kPrimaryMouseButton);
        final index = switch (control.key) {
          'next' => 2,
          'previous' => 0,
          _ => 1,
        };
        final state = control.key == 'play/pause'
            ? PlaybackState.paused
            : PlaybackState.playing;
        await _waitFor(
          tester,
          () => status?.state == state && status?.itemRequest == items[index],
          'primary ${control.key}',
        );
        expect(status!.index, index);
        expect(failures, isEmpty);
      }
    } finally {
      try {
        await tester.pumpWidget(const SizedBox.shrink());
      } finally {
        await statuses.cancel();
        await errors.cancel();
        finalizeRust();
        await directory.delete(recursive: true);
      }
    }
  }, timeout: const Timeout(Duration(minutes: 3)));
}

class _TestPaths extends PathProviderPlatform {
  _TestPaths(this.path);
  final String path;

  @override
  Future<String> getApplicationDocumentsPath() async => path;
  @override
  Future<String> getApplicationSupportPath() async => path;
  @override
  Future<String> getTemporaryPath() async => path;
}

Future<void> _mouseClick(
  WidgetTester tester,
  Finder control,
  int buttons,
) async {
  final gesture = await tester.startGesture(
    tester.getCenter(control),
    kind: PointerDeviceKind.mouse,
    buttons: buttons,
  );
  await gesture.up();
  await tester.pump();
}

Future<void> _waitFor(
  WidgetTester tester,
  bool Function() condition,
  String description,
) async {
  final deadline = DateTime.now().add(const Duration(seconds: 30));
  while (DateTime.now().isBefore(deadline)) {
    if (condition()) return;
    await tester.pump(const Duration(milliseconds: 50));
  }
  fail('Timed out waiting for $description');
}

Future<void> _load(List<PlayingItemRef> items) async {
  final response = await awaitResponse(
    'LoadResolvedItemsRequest',
    LoadResolvedItemsResponse.rustSignalStream,
    responseCallId: (event) => event.message.callId?.toInt(),
    send: (callId) => LoadResolvedItemsRequest(
      items: items,
      startIndex: 1,
      mode: PlaylistOperateMode.replace,
      playbackMode: PlaybackMode.sequential,
      positionSeconds: 0,
      play: LoadPlay.play,
      callId: callId,
    ).sendSignalToRust(),
  );
  expect(response.message.result.leftOutItems, isEmpty);
}

Future<List<PlayingItemRefIndependentFile>> _silentTracks(
  Directory directory,
) async {
  // Three minutes of 8 kHz mono PCM silence prevents natural queue advancement.
  const sampleRate = 8000;
  const dataSize = sampleRate * 2 * 180;
  final wav = Uint8List(44 + dataSize);
  final header = ByteData.sublistView(wav);
  wav.setAll(0, 'RIFF'.codeUnits);
  header.setUint32(4, wav.length - 8, Endian.little);
  wav.setAll(8, 'WAVEfmt '.codeUnits);
  header.setUint32(16, 16, Endian.little);
  header.setUint16(20, 1, Endian.little);
  header.setUint16(22, 1, Endian.little);
  header.setUint32(24, sampleRate, Endian.little);
  header.setUint32(28, sampleRate * 2, Endian.little);
  header.setUint16(32, 2, Endian.little);
  header.setUint16(34, 16, Endian.little);
  wav.setAll(36, 'data'.codeUnits);
  header.setUint32(40, dataSize, Endian.little);
  final items = <PlayingItemRefIndependentFile>[];
  for (var index = 0; index < 3; index++) {
    final file = File('${directory.path}/track-$index.wav');
    await file.writeAsBytes(wav);
    items.add(PlayingItemRefIndependentFile(path: file.path));
  }
  return items;
}
