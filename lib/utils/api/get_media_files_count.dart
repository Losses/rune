import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<int> getMediaFilesCount() async {
  final request = GetMediaFilesCountRequest();

  try {
    final rustSignal = await awaitResponse(
      'GetMediaFilesCountRequest',
      GetMediaFilesCountResponse.rustSignalStream,
      send: () => request.sendSignalToRust(),
      timeout: const Duration(seconds: 5),
    );
    final response = rustSignal.message;

    return response.count;
  } catch (e) {
    rethrow;
  }
}
