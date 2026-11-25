import 'request_failure.dart';
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
  final response = awaitResponse(
    'FetchPlaylistOccurrencesRequest',
    FetchPlaylistOccurrencesResponse.rustSignalStream.where(
      (event) => event.message.requestId == requestId,
    ),
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => FetchPlaylistOccurrencesRequest(
      requestId: requestId,
      playlistId: playlistId,
      after: after,
      pageSize: pageSize,
      bakeCoverArts: true,
      callId: callId,
    ).sendSignalToRust(),
  );
  final page = (await response).message;
  return PlaylistOccurrencePage(
    page.entries.map((entry) {
      final file = entry.file;
      return InternalMediaFile(
        id: file?.id ?? 0,
        libraryId: file?.libraryId ?? '',
        track: file?.track ?? entry.trackUuid,
        path: file?.path ?? '',
        artist: file?.artist ?? '',
        album: file?.album ?? '',
        title: file?.title ?? entry.trackUuid,
        duration: file?.duration ?? 0,
        coverArtPath: page.coverArtMap[file?.id] ?? '',
        trackNumber: file?.trackNumber,
        audioAvailability:
            file?.audioAvailability ?? AudioAvailability.unavailable,
        audioOrigin: file?.audioOrigin ?? AudioOrigin.remote,
        playlistOccurrenceId: entry.occurrenceId,
        playlistPosition: entry.position,
      );
    }).toList(),
    page.next,
  );
}
