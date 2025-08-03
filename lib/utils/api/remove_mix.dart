import '../../bindings/bindings.dart';

Future<bool> removeMix(int mixId) async {
  final updateRequest = RemoveMixRequest(mixId: mixId);
  updateRequest.sendSignalToRust(); // GENERATED

  final rustSignal = await RemoveMixResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response.success;
}
