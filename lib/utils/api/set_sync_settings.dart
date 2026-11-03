import '../../bindings/bindings.dart';

int _sequence = 0;

Future<void> setSyncSettings(
  bool listen,
  MissingHostFilePolicy missingHostFile,
) async {
  final id = '${DateTime.now().microsecondsSinceEpoch}-${_sequence++}';
  final response = SetSyncSettingsResponse.rustSignalStream
      .firstWhere((event) => event.message.requestId == id)
      .timeout(const Duration(seconds: 15));
  SetSyncSettingsRequest(
    requestId: id,
    listen: listen,
    missingHostFile: missingHostFile,
  ).sendSignalToRust();
  final error = (await response).message.error;
  if (error != null) throw StateError(error);
}
