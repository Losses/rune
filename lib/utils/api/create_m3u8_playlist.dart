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
    send: () => CreateM3u8PlaylistRequest(
      name: name,
      group: group.isEmpty ? 'Favorite' : group,
      path: path,
    ).sendSignalToRust(),
  );
  final result = rustSignal.message;

  return result;
}
