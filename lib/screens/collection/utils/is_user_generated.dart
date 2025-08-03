import '../../../bindings/bindings.dart';

bool userGenerated(CollectionType collectionType) {
  return collectionType == CollectionType.playlist ||
      collectionType == CollectionType.mix;
}
