import 'package:player/messages/artist.pbserver.dart';

Future<List<(int, String)>> fetchArtistSummary() async {
  SearchArtistSummaryRequest(n: 50).sendSignalToRust();
  return (await SearchArtistSummaryResponse.rustSignalStream.first)
      .message
      .result
      .map((x) => (x.id, x.name))
      .toList();
}
