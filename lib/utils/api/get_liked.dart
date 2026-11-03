import 'request_failure.dart';
import '../../bindings/bindings.dart';
import '../playing_item.dart';

Future<bool> getLiked(PlayingItem item) async {
  final updateRequest = GetLikedRequest(item: item.toRequest());

  final rustSignal = await awaitResponse(
    'GetLikedRequest',
    GetLikedResponse.rustSignalStream,
    send: () => updateRequest.sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response.liked;
}
