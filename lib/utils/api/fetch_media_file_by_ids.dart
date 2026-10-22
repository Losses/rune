import 'dart:async';

import '../../widgets/track_list/utils/internal_media_file.dart';
import '../../bindings/bindings.dart';
import '../order_by_ids.dart';

Future<List<InternalMediaFile>> fetchMediaFileByIds(
  List<int> ids,
  bool bakeCoverArts,
) async {
  final request = FetchMediaFileByIdsRequest(
    ids: ids,
    bakeCoverArts: true,
  );
  request.sendSignalToRust(); // GENERATED

  final response =
      (await FetchMediaFileByIdsResponse.rustSignalStream.first).message;

  return orderByIds(ids, response.mediaFiles, (x) => x.id)
      .map(
        (x) => InternalMediaFile(
          id: x.id,
          path: x.path,
          artist: x.artist,
          album: x.album,
          title: x.title,
          duration: x.duration,
          coverArtPath: response.coverArtMap[x.id] ?? '',
          trackNumber: x.trackNumber,
        ),
      )
      .toList();
}
