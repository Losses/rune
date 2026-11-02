import '../bindings/bindings.dart';

const Map<CollectionType, String> typeToOperator = {
  CollectionType.album: "lib::album",
  CollectionType.artist: "lib::artist",
  CollectionType.playlist: "lib::playlist",
  CollectionType.track: "lib::track",
  CollectionType.genre: "lib::genre",
};

List<(String, String)> buildCollectionQuery(
  CollectionType collectionType,
  int id,
) {
  if (collectionType == CollectionType.mix) {
    throw "Not Allow";
  }
  return [(typeToOperator[collectionType]!, id.toString())];
}
