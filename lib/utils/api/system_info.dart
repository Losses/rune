import 'package:rune/messages/system.pb.dart';

Future<SystemInfoResponse> systemInfo() async {
  final updateRequest = SystemInfoRequest();
  updateRequest.sendSignalToRust(); // GENERATED

  final rustSignal = await SystemInfoResponse.rustSignalStream.first;
  final response = rustSignal.message;

  return response;
}
