import 'dart:async';

import '../../messages/all.dart';

Future<SearchForResponse> searchFor(String query) async {
  if (!query.endsWith('*')) {
    query = '$query*';
  }

  final searchRequest = SearchForRequest(queryStr: query, n: 30);
  searchRequest.sendSignalToRust(); // GENERATED

  return (await SearchForResponse.rustSignalStream.first).message;
}
