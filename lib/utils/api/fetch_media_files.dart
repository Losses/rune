import 'request_failure.dart';
import '../../bindings/bindings.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';

Future<List<InternalMediaFile>> fetchMediaFiles(
  int cursor,
  int pageSize,
) async {
  final fetchMediaFiles = FetchMediaFilesRequest(
    cursor: cursor,
    pageSize: pageSize,
    bakeCoverArts: true,
  );

  // Listen for the response from Rust
  final rustSignal = await awaitResponse(
    'FetchMediaFilesRequest',
    FetchMediaFilesResponse.rustSignalStream,
    send: () => fetchMediaFiles.sendSignalToRust(),
  );
  final response = rustSignal.message;

  return response.mediaFiles
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
