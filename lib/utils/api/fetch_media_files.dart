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
  fetchMediaFiles.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await FetchMediaFilesResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.mediaFiles
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
