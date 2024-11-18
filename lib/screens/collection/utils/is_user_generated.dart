import '../../../messages/collection.pb.dart';

bool userGenerated(CollectionType collectionType) {
  return collectionType == CollectionType.Playlist ||
      collectionType == CollectionType.Mix;
}
