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
  fetchGroupsRequest.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await FetchCollectionGroupsResponse.rustSignalStream.first;
  final groups = rustSignal.message.groups;

  return groups;
}
