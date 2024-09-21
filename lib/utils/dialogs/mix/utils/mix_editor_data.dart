import 'package:player/utils/api/fetch_albums_by_ids.dart';
import 'package:player/utils/api/fetch_artists_by_ids.dart';
import 'package:player/utils/api/fetch_playlists_by_ids.dart';
import 'package:player/utils/api/fetch_track_by_ids.dart';

class MixEditorData {
  final String title;
  final String group;
  final List<(int, String)> artists;
  final List<(int, String)> albums;
  final List<(int, String)> playlists;
  final List<(int, String)> tracks;
  final Set<String> directories;
  final double limit;
  final String mode;
  final String recommendation;
  final String sortBy;
  final bool likedOnly;

  MixEditorData({
    required this.title,
    required this.group,
    this.artists = const [],
    this.albums = const [],
    this.playlists = const [],
    this.tracks = const [],
    this.directories = const {},
    required this.limit,
    required this.mode,
    required this.recommendation,
    required this.sortBy,
    required this.likedOnly,
  });

  MixEditorData copyWith({
    String? title,
    String? group,
    List<(int, String)>? artists,
    List<(int, String)>? albums,
    List<(int, String)>? playlists,
    List<(int, String)>? tracks,
    Set<String>? directories,
    double? limit,
    String? mode,
    String? recommendation,
    String? sortBy,
    bool? likedOnly,
  }) {
    return MixEditorData(
      title: title ?? this.title,
      group: group ?? this.group,
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
    title: $title,
    group: $group,
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

Future<MixEditorData> queryToMixEditorData(
  String title,
  String group,
  List<(String, String)> query,
) async {
  List<int> artistsId = [];
  List<int> albumsId = [];
  List<int> playlistsId = [];
  List<int> tracksId = [];
  Set<String> directories = {};
  double limit = 0.0;
  String mode = '99';
  String recommendation = '';
  String sortBy = 'default';
  bool likedOnly = false;

  for (var item in query) {
    switch (item.$1) {
      case 'lib::artist':
        artistsId.add(int.parse(item.$2));
        break;
      case 'lib::album':
        albumsId.add(int.parse(item.$2));
        break;
      case 'lib::playlist':
        playlistsId.add(int.parse(item.$2));
        break;
      case 'lib::track':
        tracksId.add(int.parse(item.$2));
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

  final List<(int, String)> artists =
      (await fetchArtistsByIds(artistsId)).map((x) => (x.id, x.name)).toList();
  List<(int, String)> albums =
      (await fetchAlbumsByIds(albumsId)).map((x) => (x.id, x.name)).toList();
  List<(int, String)> playlists = (await fetchPlaylistsByIds(playlistsId))
      .map((x) => (x.id, x.name))
      .toList();
  List<(int, String)> tracks =
      (await fetchTrackByIds(tracksId)).map((x) => (x.id, x.title)).toList();

  return MixEditorData(
    title: title,
    group: group,
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
    query.add(('lib::artist', artist.$1.toString()));
  }

  for (var album in data.albums) {
    query.add(('lib::album', album.$1.toString()));
  }

  for (var playlist in data.playlists) {
    query.add(('lib::playlist', playlist.$1.toString()));
  }

  for (var track in data.tracks) {
    query.add(('lib::track', track.$1.toString()));
  }

  for (var directory in data.directories) {
    query.add(('lib::directory.deep', directory));
  }

  if (data.likedOnly) {
    query.add(('filter::liked', 'true'));
  }

  if (data.limit > 0) {
    query.add(('pipe::limit', data.limit.round().toString()));
  }

  if (data.recommendation != '') {
    query.add(('pipe::recommend', data.recommendation));
  }

  if (data.sortBy != 'default') {
    query.add(('sort::${data.sortBy}', 'true'));
  }

  return query;
}
