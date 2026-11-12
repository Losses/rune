import 'request_failure.dart';
import 'dart:async';

import '../../bindings/bindings.dart';

Future<List<CollectionGroupSummary>> fetchCollectionGroupSummary(
  CollectionType collectionType,
) async {
  final request = FetchCollectionGroupSummaryRequest(
    collectionType: collectionType,
  );

  return (await awaitResponse(
    'FetchCollectionGroupSummaryRequest',
    CollectionGroupSummaryResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => request.copyWith(callId: () => callId).sendSignalToRust(),
  )).message.groups;
}
