import '../../bindings/bindings.dart';

import '../playing_item.dart';

Future<int?> getPrimaryColor(PlayingItem playingItem) async {
  GetPrimaryColorByTrackIdRequest(item: playingItem.toRequest())
      .sendSignalToRust();

  while (true) {
    final rustSignal =
        await GetPrimaryColorByTrackIdResponse.rustSignalStream.first;

    final newItem = PlayingItem.fromRequest(rustSignal.message.item);
    if (newItem == playingItem) {
      final color = rustSignal.message.primaryColor;
      if (color == 0) return null;
      return color;
    }
  }
}
