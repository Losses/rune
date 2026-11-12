import 'request_failure.dart';
import '../../bindings/bindings.dart';
import '../playing_item.dart';

Future<bool> getLiked(PlayingItem item) async {
  final updateRequest = GetLikedRequest(
    item: (await playingItemRefs([item])).single,
  );

  final rustSignal = await awaitResponse(
    'GetLikedRequest',
    GetLikedResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        updateRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response.liked;
}
