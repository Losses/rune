import 'package:flutter_test/flutter_test.dart';

import '../bindings/bindings.dart';
import '../utils/api/operate_playback_with_mix_query.dart';

void main() {
  LoadResolvedItemsRequest build(PlaybackMode? mode) =>
      loadResolvedItemsRequest(
        items: const [],
        startIndex: 0,
        operateMode: PlaylistOperateMode.replace,
        expectedRevision: null,
        playbackMode: mode,
      );

  test('a load that keeps the playback mode carries no mode', () {
    final request = build(null);
    expect(request.playbackMode, isNull);
    expect(request.play, LoadPlay.auto);
    final decoded = LoadResolvedItemsRequest.bincodeDeserialize(
      request.bincodeSerialize(),
    );
    expect(decoded.playbackMode, isNull);
  });

  test('a load that names a playback mode carries that mode', () {
    for (final mode in PlaybackMode.values) {
      final decoded = LoadResolvedItemsRequest.bincodeDeserialize(
        build(mode).bincodeSerialize(),
      );
      expect(decoded.playbackMode, mode);
    }
  });
}
