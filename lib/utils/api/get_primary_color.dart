import '../../messages/all.dart';

Future<int?> getPrimaryColor(int trackId) async {
  GetPrimaryColorByTrackIdRequest(id: trackId).sendSignalToRust();

  while (true) {
    final rustSignal =
        await GetPrimaryColorByTrackIdResponse.rustSignalStream.first;

    if (rustSignal.message.id == trackId) {
      final color = rustSignal.message.primaryColor;
      if (color == 0) return null;
      return color;
    }
  }
}
