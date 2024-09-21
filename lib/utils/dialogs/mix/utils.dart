import 'package:player/messages/mix.pbserver.dart';

Future<List<String>> getGroupList() async {
  final fetchGroupsRequest = FetchMixesGroupSummaryRequest();
  fetchGroupsRequest.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await MixGroupSummaryResponse.rustSignalStream.first;
  final groups =
      rustSignal.message.mixesGroups.map((group) => group.groupTitle).toList();

  return groups;
}

Future<MixWithoutCoverIds> getMixById(int mixId) async {
  final fetchMediaFiles = GetMixByIdRequest(mixId: mixId);
  fetchMediaFiles.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await GetMixByIdResponse.rustSignalStream.first;
  final mix = rustSignal.message.mix;

  return mix;
}

Future<List<(String, String)>> fetchMixQueriesByQueryId(int mixId) async {
  final fetchMediaFiles = FetchMixQueriesRequest(mixId: mixId);
  fetchMediaFiles.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await FetchMixQueriesResponse.rustSignalStream.first;
  final queries = rustSignal.message.result;

  return queries.map((x) => (x.operator, x.parameter)).toList();
}

Future<MixWithoutCoverIds> createMix(
  String name,
  String group,
  bool scriptletMode,
  int mode,
  Iterable<(String, String)> queries,
) async {
  final createRequest = CreateMixRequest(
    name: name,
    group: group,
    scriptletMode: scriptletMode,
    mode: mode,
    queries: queries.map((x) => MixQuery(operator: x.$1, parameter: x.$2)),
  );
  createRequest.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await CreateMixResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.mix;
}

Future<MixWithoutCoverIds> updateMix(
  int mixId,
  String name,
  String group,
  bool scriptletMode,
  int mode,
  Iterable<(String, String)> queries,
) async {
  final updateRequest = UpdateMixRequest(
    mixId: mixId,
    name: name,
    group: group,
    scriptletMode: scriptletMode,
    mode: mode,
    queries: queries.map((x) => MixQuery(operator: x.$1, parameter: x.$2)),
  );
  updateRequest.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await UpdateMixResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.mix;
}
