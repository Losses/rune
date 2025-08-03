import '../../bindings/bindings.dart';

Future<List<String>> fetchCollectionGroupSummaryTitle(
    CollectionType collectionType) async {
  final fetchGroupsRequest =
      FetchCollectionGroupSummaryRequest(collectionType: collectionType);
  fetchGroupsRequest.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await CollectionGroupSummaryResponse.rustSignalStream.first;
  final groups =
      rustSignal.message.groups.map((group) => group.groupTitle).toList();

  return groups;
}
