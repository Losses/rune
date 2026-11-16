import 'dart:async';

import 'package:flutter_test/flutter_test.dart';

import '../bindings/bindings.dart';
import '../utils/api/play_mode.dart';

void main() {
  test('sends the playback mode before a pending settings write', () {
    final pendingWrite = Completer<void>();
    final sent = <PlaybackMode>[];

    playMode(PlaybackMode.shuffle, persist: (_) => pendingWrite.future, send: sent.add);

    expect(sent, [PlaybackMode.shuffle]);
  });
}
