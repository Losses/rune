import 'package:player/messages/playlist.pbserver.dart';

Future<List<(int, String)>> fetchPlaylistSummary() async {
  SearchPlaylistSummaryRequest(n: 50).sendSignalToRust();
  return (await SearchPlaylistSummaryResponse.rustSignalStream.first)
      .message
      .result
      .map((x) => (x.id, x.name))
      .toList();
}
