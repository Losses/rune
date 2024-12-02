class InternalMediaFile {
  final int id;
  final String path;
  final String artist;
  final String album;
  final String title;
  final double duration;
  final String coverArtPath;
  final int? trackNumber;

  InternalMediaFile({
    required this.id,
    required this.path,
    required this.artist,
    required this.album,
    required this.title,
    required this.duration,
    required this.coverArtPath,
    required this.trackNumber,
  });
}
