import 'dart:async';

import '../../bindings/bindings.dart';
import '../playing_item.dart';
import 'request_failure.dart';

Future<List<LyricContentLine>> getLyricByTrackId(PlayingItem? item) async {
  final itemRef = item == null ? null : (await playingItemRefs([item])).single;
  if (item == null) return [];
  try {
    final response = await awaitResponse(
      'GetLyricByTrackIdRequest',
      GetLyricByTrackIdResponse.rustSignalStream.where(
        (event) => PlayingItem.fromRequest(event.message.item) == item,
      ),
      timeout: const Duration(seconds: 10),
      responseCallId: (response) => response.message.callId?.toInt(),
      send: (callId) => GetLyricByTrackIdRequest(
        item: itemRef!,
        callId: callId,
      ).sendSignalToRust(),
    );
    return response.message.lines;
  } on TimeoutException {
    return [];
  } on RequestFailure catch (failure) {
    if (failure.kind == RequestErrorKind.notFound) return [];
    rethrow;
  }
}
