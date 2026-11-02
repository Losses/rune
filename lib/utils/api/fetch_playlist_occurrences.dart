import 'package:uuid/uuid.dart';

import '../../bindings/bindings.dart';
import '../../widgets/track_list/utils/internal_media_file.dart';

class PlaylistOccurrencePage {
  final List<InternalMediaFile> items;
  final PlaylistOccurrenceCursor? next;
  PlaylistOccurrencePage(this.items, this.next);
}

Future<PlaylistOccurrencePage> fetchPlaylistOccurrences(
  int playlistId,
  PlaylistOccurrenceCursor? after,
  int pageSize,
) async {
  final requestId = const Uuid().v4();
  final response = FetchPlaylistOccurrencesResponse.rustSignalStream.firstWhere(
    (signal) => signal.message.requestId == requestId,
  );
  FetchPlaylistOccurrencesRequest(
    requestId: requestId,
    playlistId: playlistId,
    after: after,
    pageSize: pageSize,
    bakeCoverArts: true,
  ).sendSignalToRust();
  final page = (await response).message;
  return PlaylistOccurrencePage(
    page.entries.map((entry) {
      final file = entry.file;
      return InternalMediaFile(
        id: file?.id ?? 0,
        path: file?.path ?? '',
        artist: file?.artist ?? '',
        album: file?.album ?? '',
        title: file?.title ?? entry.trackUuid,
        duration: file?.duration ?? 0,
        coverArtPath: page.coverArtMap[file?.id] ?? '',
        trackNumber: file?.trackNumber,
        audioAvailability:
            file?.audioAvailability ?? AudioAvailability.unavailable,
        playlistOccurrenceId: entry.occurrenceId,
        playlistPosition: entry.position,
      );
    }).toList(),
    page.next,
  );
}
