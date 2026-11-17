import 'dart:io';
import 'dart:typed_data';

import 'package:bitsdojo_window/bitsdojo_window.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:macos_window_utils/macos_window_utils.dart';
import 'package:rinf/rinf.dart';

import 'package:rune/bindings/bindings.dart';
import 'package:rune/utils/api/request_failure.dart';
import 'package:rune/widgets/playback_controller/next_button.dart';
import 'package:rune/widgets/playback_controller/play_pause_button.dart';
import 'package:rune/widgets/playback_controller/previous_button.dart';

// Runs the production transport widgets against the real native player.
// All native configuration, library data and silent audio belong to this test.
void main() {
  // Let the desktop engine drive frames while the checks wait for actual
  // widget and player state, including when the window is in the background.
  IntegrationTestWidgetsFlutterBinding.ensureInitialized().framePolicy =
      LiveTestWidgetsFlutterBindingFramePolicy.benchmarkLive;
  testWidgets('only primary clicks activate desktop playback controls', (
    tester,
  ) async {
    if (Platform.isMacOS) {
      await WindowManipulator.initialize();
      await WindowManipulator.enableFullSizeContentView();
    }
    doWhenWindowReady(() {
      appWindow.size = const Size(640, 360);
      appWindow.show();
    });
    await initializeRust(assignRustSignal);
    final directory = await Directory.systemTemp.createTemp('rune-input-');
    final status = ValueNotifier<PlaybackStatus?>(null);
    final statuses = PlaybackStatus.rustSignalStream.listen(
      (event) => status.value = event.message,
    );
    final failures = <RequestFailedResponse>[];
    final errors = RequestFailedResponse.rustSignalStream.listen(
      (event) => failures.add(event.message),
    );
    try {
      final config = await Directory('${directory.path}/config').create();
      final library = await Directory('${directory.path}/library').create();
      await awaitResponse(
        'InitializeNodeRequest',
        InitializeNodeResponse.rustSignalStream,
        responseCallId: (event) => event.message.callId?.toInt(),
        send: (callId) => InitializeNodeRequest(
          configPath: config.path,
          callId: callId,
        ).sendSignalToRust(),
      );
      final registered = await awaitResponse(
        'RegisterLocalLibraryRequest',
        RegisterLibraryResponse.rustSignalStream,
        responseCallId: (event) => event.message.callId?.toInt(),
        send: (callId) => RegisterLocalLibraryRequest(
          path: library.path,
          alias: 'Input test',
          mode: LibraryInitializeMode.portable,
          callId: callId,
        ).sendSignalToRust(),
      );
      await awaitResponse(
        'OpenLibraryRequest',
        OpenLibraryResponse.rustSignalStream,
        responseCallId: (event) => event.message.callId?.toInt(),
        send: (callId) => OpenLibraryRequest(
          recordId: registered.message.record!.recordId,
          callId: callId,
        ).sendSignalToRust(),
      );
      const SetAdaptiveSwitchingEnabledRequest(
        enabled: false,
      ).sendSignalToRust();
      await tester.pumpWidget(
        FluentApp(
          home: Center(
            child: ValueListenableBuilder<PlaybackStatus?>(
              valueListenable: status,
              builder: (context, current, child) => Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  PreviousButton(
                    disabled: current?.ready != true,
                    shadows: null,
                  ),
                  PlayPauseButton(
                    disabled: current?.ready != true,
                    state: current?.state ?? PlaybackState.stopped,
                    shadows: null,
                  ),
                  NextButton(disabled: current?.ready != true, shadows: null),
                ],
              ),
            ),
          ),
        ),
      );
      final items = await _silentTracks(library);
      final controls = {
        'play/pause': find.byType(PlayPauseButton),
        'next': find.byType(NextButton),
        'previous': find.byType(PreviousButton),
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
              status.value?.ready == true &&
              status.value?.state == PlaybackState.playing &&
              status.value?.itemRequest == items[1] &&
              tester.widget<PlayPauseButton>(controls['play/pause']!).state ==
                  PlaybackState.playing &&
              !tester.widget<PlayPauseButton>(controls['play/pause']!).disabled,
          'the middle track and enabled controls',
        );
        final before = status.value!.progressSeconds;
        await _mouseClick(tester, control.value, kSecondaryMouseButton);
        // Observe actual playback progress while requiring unchanged transport
        // state; native request types have independent receivers, so another
        // request's response would not be an ordering barrier.
        await _waitFor(tester, () {
          expect(
            status.value!.state,
            PlaybackState.playing,
            reason: 'secondary ${control.key} changed playback state',
          );
          expect(
            status.value!.itemRequest,
            items[1],
            reason: 'secondary ${control.key} changed the track',
          );
          expect(failures, isEmpty);
          return status.value!.progressSeconds > before + 0.5;
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
          () =>
              status.value?.state == state &&
              status.value?.itemRequest == items[index],
          'primary ${control.key}',
        );
        expect(status.value!.index, index);
        expect(failures, isEmpty);
      }
    } finally {
      try {
        await tester.pumpWidget(const SizedBox.shrink());
      } finally {
        await statuses.cancel();
        await errors.cancel();
        status.dispose();
        finalizeRust();
        await directory.delete(recursive: true);
      }
    }
  }, timeout: const Timeout(Duration(minutes: 3)));
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
