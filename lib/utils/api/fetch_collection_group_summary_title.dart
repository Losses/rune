import 'request_failure.dart';
import '../../bindings/bindings.dart';

/// The titles of the groups a collection of [collectionType] can be put in,
/// which pickers offer. A group the library fills itself is left out.
Future<List<String>> fetchCollectionGroupSummaryTitle(
  CollectionType collectionType,
) async {
  final fetchGroupsRequest = FetchCollectionGroupSummaryRequest(
    collectionType: collectionType,
  );

  // Listen for the response from Rust
  final rustSignal = await awaitResponse(
    'FetchCollectionGroupSummaryRequest',
    CollectionGroupSummaryResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) =>
        fetchGroupsRequest.copyWith(callId: () => callId).sendSignalToRust(),
  );
  return pickableGroupTitles(rustSignal.message);
}

/// The titles of the groups of [summary] a collection can be put in, in order:
/// every one but those the library fills itself.
List<String> pickableGroupTitles(CollectionGroupSummaryResponse summary) => [
  for (final group in summary.groups)
    if (!summary.reservedGroups.contains(group.groupTitle)) group.groupTitle,
];
