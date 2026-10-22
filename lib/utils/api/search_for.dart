import 'dart:async';

import '../../bindings/bindings.dart';

Future<SearchForResponse> searchFor(String query) async {
  if (!query.endsWith('*')) {
    query = '$query*';
  }

  // Subscribe before sending so the response cannot be missed. A request that
  // fails in the hub sends no response; give up instead of waiting forever,
  // which would keep the search page from searching again.
  final response = Completer<SearchForResponse>();
  final subscription = SearchForResponse.rustSignalStream.listen((x) {
    if (!response.isCompleted) response.complete(x.message);
  });

  try {
    final searchRequest = SearchForRequest(queryStr: query, n: 30, fields: []);
    searchRequest.sendSignalToRust();

    return await response.future.timeout(const Duration(seconds: 10));
  } finally {
    // A timed-out request must not keep listening
    await subscription.cancel();
  }
}
