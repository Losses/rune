import 'dart:async';

import '../../bindings/bindings.dart';
import '../playing_item.dart';

Future<List<LyricContentLine>> getLyricByTrackId(PlayingItem? item) async {
  if (item == null) return [];

  // Subscribe before sending so the response cannot be missed, and only
  // accept the response that belongs to the requested item.
  final response = Completer<List<LyricContentLine>>();
  final subscription = GetLyricByTrackIdResponse.rustSignalStream.listen((x) {
    if (!response.isCompleted &&
        PlayingItem.fromRequest(x.message.item) == item) {
      response.complete(x.message.lines);
    }
  });

  GetLyricByTrackIdRequest(item: item.toRequest()).sendSignalToRust(); // GENERATED

  try {
    return await response.future.timeout(
      const Duration(seconds: 10),
      onTimeout: () => [],
    );
  } finally {
    // A timed-out request must not keep listening
    await subscription.cancel();
  }
}
