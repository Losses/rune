import 'request_failure.dart';
import '../../utils/query_list.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';
import '../../bindings/bindings.dart';
import 'operate_playback_with_mix_query.dart';

/// A page of a mix query. The queue of the player the UI shows travels with it,
/// for the queries that select it.
MixQueryRequest mixQueryRequest(
  QueryList queries,
  int? cursor,
  int? pageSize,
  List<PlaylistItem>? queue,
) {
  return MixQueryRequest(
    queries: queries.toQueryList(),
    pageSize: pageSize ?? 30,
    cursor: cursor ?? 0,
    bakeCoverArts: true,
    queue: CallerQueue(items: queueItemsToExclude(castTo: null, queue: queue)),
  );
}

Future<List<InternalMediaFile>> queryMixTracks(
  QueryList queries, [
  int? cursor,
  int? pageSize,
]) async {
  final request = mixQueryRequest(
    queries,
    cursor,
    pageSize,
    PlaylistUpdate.latestRustSignal?.message.items,
  );

  final response = (await awaitResponse(
    'MixQueryRequest',
    MixQueryResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => request.copyWith(callId: () => callId).sendSignalToRust(),
  )).message;

  return response.files
      .map(
        (x) => InternalMediaFile(
          id: x.id,
          libraryId: x.libraryId,
          track: x.track,
          path: x.path,
          artist: x.artist,
          album: x.album,
          title: x.title,
          duration: x.duration,
          coverArtPath: response.coverArtMap[x.id] ?? '',
          trackNumber: x.trackNumber,
          audioAvailability: x.audioAvailability,
          audioOrigin: x.audioOrigin,
        ),
      )
      .toList();
}
