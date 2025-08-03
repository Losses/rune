import '../../bindings/bindings.dart';

Future<Mix> createMix(
  String name,
  String group,
  bool scriptletMode,
  int mode,
  Iterable<(String, String)> queries,
) async {
  final createRequest = CreateMixRequest(
    name: name,
    group: group.isEmpty ? 'Favorite' : group,
    scriptletMode: scriptletMode,
    mode: mode,
    queries:
        queries.map((x) => MixQuery(operator: x.$1, parameter: x.$2)).toList(),
  );
  createRequest.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await CreateMixResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.mix;
}
