import '../../bindings/bindings.dart';
import '../playing_item.dart';

Future<bool> getLiked(PlayingItem item) async {
  final updateRequest = GetLikedRequest(item: item.toRequest());
  updateRequest.sendSignalToRust(); // GENERATED

  final rustSignal = await GetLikedResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.liked;
}
