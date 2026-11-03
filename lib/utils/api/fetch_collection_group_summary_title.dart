import 'request_failure.dart';
import '../../bindings/bindings.dart';

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
    send: () => fetchGroupsRequest.sendSignalToRust(),
  );
  final groups = rustSignal.message.groups
      .map((group) => group.groupTitle)
      .toList();

  return groups;
}
