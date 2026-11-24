import '../../bindings/bindings.dart';

final Map<CollectionType, String> routerName = {
  CollectionType.album: 'albums',
  CollectionType.artist: 'artists',
  CollectionType.playlist: 'playlists',
  CollectionType.mix: 'mixes',
  CollectionType.genre: 'genres',
};

collectionTypeToRouterName(CollectionType x) => routerName[x];
