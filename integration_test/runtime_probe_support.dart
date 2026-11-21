import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:rune/providers/playlist.dart';
import 'package:rune/providers/status.dart';

Future<void> probeTick(WidgetTester tester) async {
  await Future<void>.delayed(const Duration(milliseconds: 200));
  await tester.pump();
}

Future<void> probeWait(
  WidgetTester tester,
  bool Function() condition,
  String what,
) async {
  final watch = Stopwatch()..start();
  while (!condition()) {
    if (watch.elapsed >= const Duration(seconds: 45)) {
      throw TimeoutException('Timed out waiting for $what');
    }
    await probeTick(tester);
  }
}

void probeRequire(bool condition, String message) {
  if (!condition) throw StateError(message);
}

Map<String, Object?> statusSnapshot(PlaybackStatusProvider provider) {
  final s = provider.playbackStatus;
  return {
    'state': s.state.toString(),
    'ready': s.ready,
    'title': s.title,
    'item': s.item,
    'playing_item': provider.playingItem?.toString(),
    'index': s.index,
    'progress_seconds': s.progressSeconds,
    'duration_seconds': s.duration,
    'playback_mode': s.playbackMode,
  };
}

List<Map<String, Object?>> queueSnapshot(PlaylistProvider provider) =>
    provider.items
        .map(
          (e) => <String, Object?>{
            'index': e.index,
            'title': e.entry.title,
            'artist': e.entry.artist,
            'item': e.item.toString(),
            'unavailable': e.entry.unavailable?.toString(),
          },
        )
        .toList();

class ProbeEvidence {
  ProbeEvidence(this.file, List<String> names)
    : steps = {for (final n in names) n: 'pending'} {
    _flush();
  }

  final File file;
  final Map<String, String> steps;
  final List<Map<String, Object?>> events = [];
  String result = 'running';
  String? current;

  void _flush() => file.writeAsStringSync(
    const JsonEncoder.withIndent(
      '  ',
    ).convert({'result': result, 'steps': steps, 'events': events}),
    flush: true,
  );

  void event(String name, Map<String, Object?> data) {
    events.add({
      'time_utc': DateTime.now().toUtc().toIso8601String(),
      'event': name,
      ...data,
    });
    _flush();
  }

  Future<void> step(
    String name,
    Future<Map<String, Object?>> Function() action,
  ) async {
    current = name;
    steps[name] = 'running';
    event('step_started', {'step': name});
    final data = await action();
    steps[name] = 'pass';
    event('step_passed', {'step': name, ...data});
    current = null;
  }

  void fail(Object error, StackTrace stack, Map<String, Object?> data) {
    if (current != null) steps[current!] = 'fail';
    result = 'fail';
    event('failure', {
      'step': current,
      'error': error.toString(),
      'stack': stack.toString(),
      ...data,
    });
    current = null;
  }

  void complete() {
    result = 'pass';
    event('complete', {});
  }

  void finishIfIncomplete() {
    if (result == 'running') {
      if (current != null) steps[current!] = 'fail';
      result = 'fail';
      event('incomplete_teardown', {'step': current});
    }
  }
}
