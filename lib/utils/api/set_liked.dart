import '../../bindings/bindings.dart';
import '../playing_item.dart';

Future<bool?> setLiked(PlayingItem item, bool liked) async {
  final updateRequest = SetLikedRequest(
    item: item.toRequest(),
    liked: liked,
  );
  updateRequest.sendSignalToRust(); // GENERATED

  final rustSignal = await SetLikedResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.success ? liked : null;
}
