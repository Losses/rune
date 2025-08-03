import '../../bindings/bindings.dart';

Future<Mix> getMixById(int mixId) async {
  final fetchMediaFiles = GetMixByIdRequest(mixId: mixId);
  fetchMediaFiles.sendSignalToRust(); // GENERATED

  // Listen for the response from Rust
  final rustSignal = await GetMixByIdResponse.rustSignalStream.first;
  final mix = rustSignal.message.mix;

  return mix;
}
