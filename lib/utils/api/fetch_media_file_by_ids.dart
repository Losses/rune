import 'dart:async';
import 'package:player/messages/media_file.pb.dart';

Future<List<MediaFile>> fetchMediaFileByIds(List<int> ids) async {
  final request = FetchMediaFileByIdsRequest(ids: ids);
  request.sendSignalToRust(); // GENERATED

  return (await FetchMediaFileByIdsResponse.rustSignalStream.first)
      .message
      .result;
}
