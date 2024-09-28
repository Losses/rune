import 'package:player/messages/media_file.pb.dart';
import 'package:player/widgets/track_list/track_list.dart';

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
            coverArtPath: response.coverArtMap[x.id] ?? ''),
      )
      .toList();
}
