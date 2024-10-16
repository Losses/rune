import 'package:rune/messages/mix.pbserver.dart';

Future<Mix> updateMix(
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
