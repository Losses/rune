import 'dart:async';

import 'package:flutter_test/flutter_test.dart';

import '../utils/api/play_mode.dart';

void main() {
  test('sends the playback mode before a pending settings write', () {
    final pendingWrite = Completer<void>();
    final sent = <int>[];

    playMode(7, persist: (_) => pendingWrite.future, send: sent.add);

    expect(sent, [7]);
  });
}
