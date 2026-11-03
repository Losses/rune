import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<List<CollectionGroup>> fetchCollectionGroups(
  CollectionType collectionType,
  List<String> groupTitles,
) async {
  final fetchGroupsRequest = FetchCollectionGroupsRequest(
    collectionType: collectionType,
    groupTitles: groupTitles,
    bakeCoverArts: true,
  );

  // Listen for the response from Rust
  final rustSignal = await awaitResponse(
    'FetchCollectionGroupsRequest',
    FetchCollectionGroupsResponse.rustSignalStream,
    send: () => fetchGroupsRequest.sendSignalToRust(),
  );
  final groups = rustSignal.message.groups;

  return groups;
}
