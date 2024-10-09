import '../messages/collection.pb.dart';

final Map<CollectionType, String> routerName = {
  CollectionType.Album: 'albums',
  CollectionType.Artist: 'artists',
  CollectionType.Playlist: 'playlists',
  CollectionType.Mix: 'mixes',
};

collectionTypeToRouterName(CollectionType x) => routerName[x];
