import 'request_failure.dart';
import 'query_mix_tracks.dart';
import 'dart:async';

import '../../bindings/bindings.dart';
import '../order_by_ids.dart';

Future<List<Collection>> fetchCollectionByIds(
  CollectionType collectionType,
  List<int> ids, {
  bool bakeCoverArts = false,
}) async {
  final request = FetchCollectionByIdsRequest(
    ids: ids,
    collectionType: collectionType,
    bakeCoverArts: bakeCoverArts,
    queue: shownPlayerQueue(),
  );

  final response = (await awaitResponse(
    'FetchCollectionByIdsRequest',
    FetchCollectionByIdsResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => request.copyWith(callId: () => callId).sendSignalToRust(),
  )).message;

  return orderByIds(ids, response.result, (x) => x.id);
}
