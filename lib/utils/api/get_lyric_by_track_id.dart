import '../../bindings/bindings.dart';
import '../playing_item.dart';

Future<List<LyricContentLine>> getLyricByTrackId(PlayingItem? item) async {
  if (item == null) return [];

  final updateRequest = GetLyricByTrackIdRequest(item: item.toRequest());
  updateRequest.sendSignalToRust(); // GENERATED

  final rustSignal = await GetLyricByTrackIdResponse.rustSignalStream.first;
  final response = rustSignal.message.lines;

  return response;
}
