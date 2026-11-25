import 'request_failure.dart';
import 'dart:async';

import '../../bindings/bindings.dart';

/// The groups of the collection page of [collectionType], with those the
/// library fills itself.
Future<CollectionGroupSummaryResponse> fetchCollectionGroupSummary(
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
  )).message;
}
