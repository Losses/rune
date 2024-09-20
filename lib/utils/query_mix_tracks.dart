import 'package:player/messages/media_file.pb.dart';

Future<List<MediaFile>> queryMixTracks(
  List<(String, String)> queries, [
  int? cursor,
  int? pageSize,
]) async {
  final request = MixQueryRequest(
    queries:
        queries.map((x) => MixQuery(operator: x.$1, parameter: x.$2)).toList(),
    pageSize: pageSize ?? 30,
    cursor: cursor,
  );
  request.sendSignalToRust(); // GENERATED

  return (await MixQueryResponse.rustSignalStream.first).message.result;
}
