import '../../bindings/bindings.dart';

Future<void> addItemToMix(int mixId, String operator, String parameter) async {
  final request = AddItemToMixRequest(
    mixId: mixId,
    operator: operator,
    parameter: parameter,
  );
  request.sendSignalToRust(); // GENERATED

  await AddItemToMixResponse.rustSignalStream.first;
}
