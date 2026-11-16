import 'dart:async';
import 'dart:typed_data';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rinf/rinf.dart';

import '../../bindings/bindings.dart';
import '../../providers/status.dart';

PlaybackStatus _status(PlaybackState state, {double progress = 0}) =>
    PlaybackStatus(
      sessionId: 'local',
      state: state,
      progressSeconds: progress,
      progressPercentage: progress / 10,
      duration: 10,
      playbackMode: PlaybackMode.sequential,
      ready: true,
      libPath: '',
    );

void main() {
  testWidgets('a status update asks for the frame that shows it', (
    tester,
  ) async {
    final messages = StreamController<RustSignalPack<PlaybackStatus>>.broadcast(
      sync: true,
    );
    final provider = PlaybackStatusProvider(statusStream: messages.stream);
    addTearDown(provider.dispose);
    addTearDown(messages.close);
    var notified = 0;
    provider.addListener(() => notified++);

    // A page where nothing moves: no frame is coming.
    await tester.pumpWidget(const SizedBox());
    expect(tester.binding.hasScheduledFrame, isFalse);

    messages.add(RustSignalPack(_status(PlaybackState.playing), Uint8List(0)));
    expect(tester.binding.hasScheduledFrame, isTrue);

    await tester.pump();
    expect(notified, 1);
    expect(provider.playbackStatus.state, PlaybackState.playing);
  });

  testWidgets('updates that arrive together reach the listeners once', (
    tester,
  ) async {
    final messages = StreamController<RustSignalPack<PlaybackStatus>>.broadcast(
      sync: true,
    );
    final provider = PlaybackStatusProvider(statusStream: messages.stream);
    addTearDown(provider.dispose);
    addTearDown(messages.close);
    var notified = 0;
    provider.addListener(() => notified++);
    await tester.pumpWidget(const SizedBox());

    messages
      ..add(RustSignalPack(_status(PlaybackState.playing), Uint8List(0)))
      ..add(
        RustSignalPack(
          _status(PlaybackState.playing, progress: 1),
          Uint8List(0),
        ),
      );
    await tester.pump();

    expect(notified, 1);
    expect(provider.playbackStatus.progressSeconds, 1);
  });
}
