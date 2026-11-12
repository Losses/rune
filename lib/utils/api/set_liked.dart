import 'request_failure.dart';
import '../../bindings/bindings.dart';
import '../playing_item.dart';

Future<bool?> setLiked(PlayingItem item, bool liked) async {
  final updateRequest = SetLikedRequest(
    item: (await playingItemRefs([item])).single,
    liked: liked,
  );
  final rustSignal = await awaitResponse(
    'SetLikedRequest',
    SetLikedResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        updateRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response.success ? liked : null;
}
