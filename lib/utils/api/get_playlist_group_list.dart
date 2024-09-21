import 'package:player/messages/playlist.pb.dart';

Future<List<String>> getPlaylistGroupList() async {
  final fetchGroupsRequest = FetchPlaylistsGroupSummaryRequest();
  fetchGroupsRequest.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await PlaylistGroupSummaryResponse.rustSignalStream.first;
  final groups = rustSignal.message.playlistsGroups
      .map((group) => group.groupTitle)
      .toList();

  return groups;
}
