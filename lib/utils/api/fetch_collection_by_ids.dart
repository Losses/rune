import 'request_failure.dart';
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

  final response = (await awaitResponse(
    'FetchCollectionByIdsRequest',
    FetchCollectionByIdsResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => request.copyWith(callId: () => callId).sendSignalToRust(),
  )).message;

  return orderByIds(ids, response.result, (x) => x.id);
}
