import '../../bindings/bindings.dart';

Future<CreateM3u8PlaylistResponse> createM3u8Playlist(
  String name,
  String group,
  String path,
) async {
  CreateM3u8PlaylistRequest(
    name: name,
    group: group.isEmpty ? 'Favorite' : group,
    path: path,
  ).sendSignalToRust();

  final rustSignal = await CreateM3u8PlaylistResponse.rustSignalStream.first;
  final result = rustSignal.message;

  return result;
}
