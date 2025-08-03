import 'dart:async';

import '../../bindings/bindings.dart';

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
