import '../../utils/query_list.dart';
import '../../messages/mix.pb.dart';
import '../../messages/media_file.pb.dart';

Future<List<MediaFile>> queryMixTracks(
  QueryList queries, [
  int? cursor,
  int? pageSize,
]) async {
  final request = MixQueryRequest(
    queries: queries.toQueryList(),
    pageSize: pageSize ?? 30,
    cursor: cursor,
  );
  request.sendSignalToRust(); // GENERATED

  return (await MixQueryResponse.rustSignalStream.first).message.result;
}
