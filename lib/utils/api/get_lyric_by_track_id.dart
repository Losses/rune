import 'dart:async';

import '../../bindings/bindings.dart';
import '../playing_item.dart';

Future<List<LyricContentLine>> getLyricByTrackId(PlayingItem? item) async {
  if (item == null) return [];

  // Subscribe before sending so the response cannot be missed, and only
  // accept the response that belongs to the requested item.
  final response = GetLyricByTrackIdResponse.rustSignalStream
      .map((x) => x.message)
      .firstWhere((x) => PlayingItem.fromRequest(x.item) == item)
      .timeout(
        const Duration(seconds: 10),
        onTimeout: () => GetLyricByTrackIdResponse(
          item: item.toRequest(),
          lines: [],
        ),
      );

  GetLyricByTrackIdRequest(item: item.toRequest()).sendSignalToRust(); // GENERATED

  return (await response).lines;
}
