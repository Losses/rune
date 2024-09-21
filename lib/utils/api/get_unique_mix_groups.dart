import 'package:player/messages/mix.pbserver.dart';

Future<List<String>> getUniqueMixGroups() async {
  GetUniqueMixGroupsRequest().sendSignalToRust();
  return (await GetUniqueMixGroupsResponse.rustSignalStream.first)
      .message
      .groups;
}
