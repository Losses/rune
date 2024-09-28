import 'dart:async';
import 'package:player/messages/media_file.pb.dart';
import 'package:player/widgets/track_list/track_list.dart';

Future<List<InternalMediaFile>> fetchMediaFileByIds(
    List<int> ids, bool bakeCoverArts) async {
  final request = FetchMediaFileByIdsRequest(
    ids: ids,
    bakeCoverArts: true,
  );
  request.sendSignalToRust(); // GENERATED

  final response =
      (await FetchMediaFileByIdsResponse.rustSignalStream.first).message;

  return response.mediaFiles
      .map(
        (x) => InternalMediaFile(
            id: x.id,
            path: x.path,
            artist: x.artist,
            album: x.album,
            title: x.title,
            duration: x.duration,
            coverArtPath: response.coverArtMap[x.id] ?? ''),
      )
      .toList();
}
