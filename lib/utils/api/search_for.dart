import '../../bindings/bindings.dart';
import 'request_failure.dart';

Future<SearchForResponse> searchFor(String query) async {
  if (!query.endsWith('*')) query = '$query*';
  final response = await awaitResponse(
    'SearchForRequest',
    SearchForResponse.rustSignalStream,
    timeout: const Duration(seconds: 10),
    send: () =>
        SearchForRequest(queryStr: query, n: 30, fields: []).sendSignalToRust(),
  );
  return response.message;
}
