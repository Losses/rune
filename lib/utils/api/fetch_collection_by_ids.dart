import 'dart:async';
import 'package:player/messages/collection.pb.dart';

Future<List<Collection>> fetchCollectionByIds(
  CollectionType collectionType,
  List<int> ids,
) async {
  final request = FetchCollectionByIdsRequest(
    ids: ids,
    collectionType: collectionType,
    bakeCoverArts: true,
  );
  request.sendSignalToRust(); // GENERATED

  return (await FetchCollectionByIdsResponse.rustSignalStream.first)
      .message
      .result;
}
