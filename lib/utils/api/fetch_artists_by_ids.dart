import 'dart:async';
import 'package:player/messages/artist.pb.dart';

Future<List<Artist>> fetchArtistsByIds(List<int> ids) async {
  final request = FetchArtistsByIdsRequest(ids: ids);
  request.sendSignalToRust(); // GENERATED

  return (await FetchArtistsByIdsResponse.rustSignalStream.first)
      .message
      .result;
}
