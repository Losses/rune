import '../../bindings/bindings.dart';

int _sequence = 0;

Future<GetSyncSettingsResponse> getSyncSettings() async {
  final id = '${DateTime.now().microsecondsSinceEpoch}-${_sequence++}';
  final response = GetSyncSettingsResponse.rustSignalStream
      .firstWhere((event) => event.message.requestId == id)
      .timeout(const Duration(seconds: 15));
  GetSyncSettingsRequest(requestId: id).sendSignalToRust();
  final result = (await response).message;
  if (result.error != null) throw StateError(result.error!);
  return result;
}
