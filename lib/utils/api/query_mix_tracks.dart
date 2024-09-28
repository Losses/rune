import 'package:player/widgets/track_list/track_list.dart';

import '../../utils/query_list.dart';
import '../../messages/mix.pb.dart';

Future<List<InternalMediaFile>> queryMixTracks(
  QueryList queries, [
  int? cursor,
  int? pageSize,
]) async {
  final request = MixQueryRequest(
    queries: queries.toQueryList(),
    pageSize: pageSize ?? 30,
    cursor: cursor,
    bakeCoverArts: true,
  );
  request.sendSignalToRust(); // GENERATED

  final response = (await MixQueryResponse.rustSignalStream.first).message;

  return response.files
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
