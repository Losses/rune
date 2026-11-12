import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<CreateM3u8PlaylistResponse> createM3u8Playlist(
  String name,
  String group,
  String path,
) async {
  final rustSignal = await awaitResponse(
    'CreateM3u8PlaylistRequest',
    CreateM3u8PlaylistResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => CreateM3u8PlaylistRequest(
      name: name,
      group: group.isEmpty ? 'Favorite' : group,
      path: path,
      callId: callId,
    ).sendSignalToRust(),
  );
  final result = rustSignal.message;

  return result;
}
