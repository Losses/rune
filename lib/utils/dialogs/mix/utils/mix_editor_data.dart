class MixEditorData {
  final String title;
  final String group;
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
    this.title = '',
    this.group = '',
    this.artists = const [],
    this.albums = const [],
    this.playlists = const [],
    this.tracks = const [],
    this.directories = const {},
    this.limit = 0.0,
    this.mode = '99',
    this.recommendation = '',
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

  @override
  String toString() {
    return '''MixEditorData(
    artists: $artists,
    albums: $albums,
    playlists: $playlists,
    tracks: $tracks,
    directories: $directories,
    limit: $limit,
    mode: $mode,
    recommendation: $recommendation,
    sortBy: $sortBy,
    likedOnly: $likedOnly,
)
''';
  }
}

MixEditorData queryToMixEditorData(List<(String, String)> query) {
  List<int> artists = [];
  List<int> albums = [];
  List<int> playlists = [];
  List<int> tracks = [];
  Set<String> directories = {};
  double limit = 0.0;
  String mode = '99';
  String recommendation = '';
  String sortBy = 'default';
  bool likedOnly = false;

  for (var item in query) {
    switch (item.$1) {
      case 'lib::artist':
        artists.add(int.parse(item.$2));
        break;
      case 'lib::album':
        albums.add(int.parse(item.$2));
        break;
      case 'lib::playlist':
        playlists.add(int.parse(item.$2));
        break;
      case 'lib::track':
        tracks.add(int.parse(item.$2));
        break;
      case 'lib::directory.deep':
        directories.add(item.$2);
        break;
      case 'filter::liked':
        likedOnly = item.$2.toLowerCase() == 'true';
        break;
      case 'pipe::limit':
        limit = double.parse(item.$2);
        break;
      case 'pipe::recommend':
        recommendation = item.$2;
        break;
      case 'sort::last_modified':
      case 'sort::duration':
      case 'sort::playedthrough':
      case 'sort::skipped':
        sortBy = item.$1.split('::')[1];
        break;
    }
  }

  return MixEditorData(
    artists: artists,
    albums: albums,
    playlists: playlists,
    tracks: tracks,
    directories: directories,
    limit: limit,
    mode: mode,
    recommendation: recommendation,
    sortBy: sortBy,
    likedOnly: likedOnly,
  );
}

List<(String, String)> mixEditorDataToQuery(MixEditorData data) {
  List<(String, String)> query = [];

  for (var artist in data.artists) {
    query.add(('lib::artist', artist.toString()));
  }

  for (var album in data.albums) {
    query.add(('lib::album', album.toString()));
  }

  for (var playlist in data.playlists) {
    query.add(('lib::playlist', playlist.toString()));
  }

  for (var track in data.tracks) {
    query.add(('lib::track', track.toString()));
  }

  for (var directory in data.directories) {
    query.add(('lib::directory.deep', directory));
  }

  if (data.likedOnly) {
    query.add(('filter::liked', 'true'));
  }

  if (data.limit > 0) {
    query.add(('pipe::limit', data.limit.toString()));
  }

  if (data.recommendation != '') {
    query.add(('pipe::recommend', data.recommendation));
  }

  if (data.sortBy != 'default') {
    query.add(('sort::${data.sortBy}', 'true'));
  }

  return query;
}
