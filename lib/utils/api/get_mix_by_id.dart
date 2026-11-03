import 'request_failure.dart';
import '../../bindings/bindings.dart';

Future<Mix> getMixById(int mixId) async {
  final fetchMediaFiles = GetMixByIdRequest(mixId: mixId);

  // Listen for the response from Rust
  final rustSignal = await awaitResponse(
    'GetMixByIdRequest',
    GetMixByIdResponse.rustSignalStream,
    send: () => fetchMediaFiles.sendSignalToRust(),
  );
  final mix = rustSignal.message.mix;

  return mix;
}
