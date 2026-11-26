import 'dart:async';

import 'package:flutter_test/flutter_test.dart';

import '../bindings/bindings.dart';

/// Waits for two native playing positions of the selected catalog item.
Future<(double, double)> trackProgress(
  Stream<PlaybackStatus> statuses,
  PlayingItemRefLibraryTrack expected,
) async {
  double? first;
  final advancing = statuses.where((status) {
    final position = status.progressSeconds;
    if (!status.ready ||
        status.state != PlaybackState.playing ||
        status.itemRequest != expected ||
        !position.isFinite ||
        position < 0) {
      first = null;
      return false;
    }
    first ??= position;
    return position > first!;
  });
  final last = await advancing
      .timeout(
        const Duration(seconds: 20),
        onTimeout: (sink) => sink.addError(
          TimeoutException(
            'No playback progress for ${expected.libraryId}/${expected.track}',
          ),
        ),
      )
      .first;
  return (first!, last.progressSeconds);
}

void main() {
  const track = PlayingItemRefLibraryTrack(
    libraryId: 'library',
    track: 'fixture',
  );

  PlaybackStatus sample(
    double progress, {
    PlayingItemRef item = track,
    PlaybackState state = PlaybackState.playing,
    bool ready = true,
  }) => PlaybackStatus(
    sessionId: 'local',
    state: state,
    progressSeconds: progress,
    progressPercentage: progress / 10,
    duration: 10,
    itemRequest: item,
    playbackMode: PlaybackMode.sequential,
    ready: ready,
    libPath: '',
  );

  for (final (name, statuses) in [
    ('empty', <PlaybackStatus>[]),
    ('one position', [sample(3)]),
    ('stalled', [sample(1), sample(1)]),
    ('paused', [sample(0), sample(1, state: PlaybackState.paused)]),
    ('not ready', [sample(0), sample(1, ready: false)]),
    (
      'wrong track',
      [sample(0), sample(1, item: track.copyWith(track: 'other'))],
    ),
    (
      'wrong library',
      [sample(0), sample(1, item: track.copyWith(libraryId: 'other'))],
    ),
    (
      'interrupted',
      [sample(0), sample(0, state: PlaybackState.stopped), sample(2)],
    ),
  ]) {
    test('$name playback evidence fails verification', () async {
      await expectLater(
        trackProgress(Stream.fromIterable(statuses), track),
        throwsStateError,
      );
    });
  }

  test('a stream error fails verification', () async {
    final error = StateError('Failed to open file');
    await expectLater(
      trackProgress(Stream.error(error), track),
      throwsA(same(error)),
    );
  });

  test('advancing positions of the intended playing track pass', () async {
    final statuses = Stream.fromIterable([
      sample(0.25),
      sample(0.25),
      sample(1.5),
    ]);
    expect(await trackProgress(statuses, track), (0.25, 1.5));
  });
}
