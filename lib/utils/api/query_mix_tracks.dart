import '../../utils/query_list.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';
import '../../bindings/bindings.dart';

Future<List<InternalMediaFile>> queryMixTracks(
  QueryList queries, [
  int? cursor,
  int? pageSize,
]) async {
  final request = MixQueryRequest(
    queries: queries.toQueryList(),
    pageSize: pageSize ?? 30,
    cursor: cursor ?? 0,
    bakeCoverArts: true,
  );
  request.sendSignalToRust();

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
          coverArtPath: response.coverArtMap[x.id] ?? '',
          trackNumber: x.trackNumber,
        ),
      )
      .toList();
}
