import '../../../bindings/bindings.dart';

class InternalMediaFile {
  final int id;
  final String libraryId;
  final String track;
  final String path;
  final String artist;
  final String album;
  final String title;
  final double duration;
  final String coverArtPath;
  final int? trackNumber;
  final AudioAvailability audioAvailability;
  final AudioOrigin audioOrigin;
  final String? playlistOccurrenceId;
  final int? playlistPosition;

  InternalMediaFile({
    required this.id,
    this.libraryId = '',
    this.track = '',
    required this.path,
    required this.artist,
    required this.album,
    required this.title,
    required this.duration,
    required this.coverArtPath,
    required this.trackNumber,
    this.audioAvailability = AudioAvailability.local,
    this.audioOrigin = AudioOrigin.host,
    this.playlistOccurrenceId,
    this.playlistPosition,
  });

  bool get canPlay =>
      audioAvailability == AudioAvailability.local ||
      audioAvailability == AudioAvailability.remoteOnly;
}
