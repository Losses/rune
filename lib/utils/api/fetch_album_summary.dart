import 'package:player/messages/album.pb.dart';

Future<List<(int, String)>> fetchAlbumSummary() async {
  SearchAlbumSummaryRequest(n: 50).sendSignalToRust();
  return (await SearchAlbumSummaryResponse.rustSignalStream.first)
      .message
      .result
      .map((x) => (x.id, x.name))
      .toList();
}
