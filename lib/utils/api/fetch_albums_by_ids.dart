import 'dart:async';
import 'package:player/messages/album.pb.dart';

Future<List<Album>> fetchAlbumsByIds(List<int> ids) async {
  final request = FetchAlbumsByIdsRequest(ids: ids);
  request.sendSignalToRust(); // GENERATED

  return (await FetchAlbumsByIdsResponse.rustSignalStream.first).message.result;
}
