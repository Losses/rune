import 'package:player/messages/media_file.pb.dart';

Future<List<MediaFile>> fetchTrackByIds(List<int> ids) async {
  FetchMediaFileByIdsRequest(ids: ids).sendSignalToRust();
  return (await FetchMediaFileByIdsResponse.rustSignalStream.first)
      .message
      .result;
}
