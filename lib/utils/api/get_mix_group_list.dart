import 'package:player/messages/mix.pbserver.dart';

Future<List<String>> getMixGroupList() async {
  final fetchGroupsRequest = FetchMixesGroupSummaryRequest();
  fetchGroupsRequest.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await MixGroupSummaryResponse.rustSignalStream.first;
  final groups =
      rustSignal.message.mixesGroups.map((group) => group.groupTitle).toList();

  return groups;
}
