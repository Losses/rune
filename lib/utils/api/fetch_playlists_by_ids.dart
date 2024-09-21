import 'dart:async';
import 'package:player/messages/playlist.pbserver.dart';

Future<List<Playlist>> fetchPlaylistsByIds(List<int> ids) async {
  final request = FetchPlaylistsByIdsRequest(ids: ids);
  request.sendSignalToRust(); // GENERATED

  return (await FetchPlaylistsByIdsResponse.rustSignalStream.first)
      .message
      .result;
}
