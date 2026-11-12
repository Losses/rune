import 'request_failure.dart';
import '../../bindings/bindings.dart';

import '../playing_item.dart';

Future<int?> getPrimaryColor(PlayingItem playingItem) async {
  final itemRef = (await playingItemRefs([playingItem])).single;
  while (true) {
    final rustSignal = await awaitResponse(
      'GetPrimaryColorByTrackIdRequest',
      GetPrimaryColorByTrackIdResponse.rustSignalStream.where(
        (event) => PlayingItem.fromRequest(event.message.item) == playingItem,
      ),
      responseCallId: (response) => response.message.callId?.toInt(),
      send: (callId) => GetPrimaryColorByTrackIdRequest(
        item: itemRef,
        callId: callId,
      ).sendSignalToRust(),
    );

    final newItem = PlayingItem.fromRequest(rustSignal.message.item);
    if (newItem == playingItem) {
      final color = rustSignal.message.primaryColor;
      if (color == 0) return null;
      return color;
    }
  }
}
