import 'package:rune/messages/media_file.pb.dart';

Future<List<(int, String)>> fetchTrackSummary() async {
  SearchMediaFileSummaryRequest(n: 50).sendSignalToRust();
  return (await SearchMediaFileSummaryResponse.rustSignalStream.first)
      .message
      .result
      .map((x) => (x.id, x.name))
      .toList();
}
