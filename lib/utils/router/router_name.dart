import '../../messages/collection.pb.dart';

final Map<CollectionType, String> routerName = {
  CollectionType.Album: 'albums',
  CollectionType.Artist: 'artists',
  CollectionType.Playlist: 'playlists',
  CollectionType.Mix: 'mixes',
  CollectionType.Genre: 'genres',
};

collectionTypeToRouterName(CollectionType x) => routerName[x];
