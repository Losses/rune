import '../../bindings/bindings.dart';

Future<int> getMediaFilesCount() async {
  final request = GetMediaFilesCountRequest();
  request.sendSignalToRust();

  try {
    final rustSignal = await GetMediaFilesCountResponse.rustSignalStream.first
        .timeout(const Duration(seconds: 5));
    final response = rustSignal.message;

    return response.count;
  } catch (e) {
    rethrow;
  }
}
