class MixEditorData {
  final List<int> artists;
  final List<int> albums;
  final List<int> playlists;
  final List<int> tracks;
  final Set<String> directories;
  final double limit;
  final String mode;
  final String recommendation;
  final String sortBy;
  final bool likedOnly;

  MixEditorData({
    this.artists = const [],
    this.albums = const [],
    this.playlists = const [],
    this.tracks = const [],
    this.directories = const {},
    this.limit = 0.0,
    this.mode = '99',
    this.recommendation = '99',
    this.sortBy = 'default',
    this.likedOnly = false,
  });

  MixEditorData copyWith({
    List<int>? artists,
    List<int>? albums,
    List<int>? playlists,
    List<int>? tracks,
    Set<String>? directories,
    double? limit,
    String? mode,
    String? recommendation,
    String? sortBy,
    bool? likedOnly,
  }) {
    return MixEditorData(
      artists: artists ?? this.artists,
      albums: albums ?? this.albums,
      playlists: playlists ?? this.playlists,
      tracks: tracks ?? this.tracks,
      directories: directories ?? this.directories,
      limit: limit ?? this.limit,
      mode: mode ?? this.mode,
      recommendation: recommendation ?? this.recommendation,
      sortBy: sortBy ?? this.sortBy,
      likedOnly: likedOnly ?? this.likedOnly,
    );
  }
}
