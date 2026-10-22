import 'dart:async';

import '../../bindings/bindings.dart';
import '../order_by_ids.dart';

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

  final response =
      (await FetchCollectionByIdsResponse.rustSignalStream.first).message;

  return orderByIds(ids, response.result, (x) => x.id);
}
