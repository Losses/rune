import 'dart:async';
import 'package:player/messages/search.pb.dart';

Future<SearchForResponse> searchFor(String query) async {
  final searchRequest = SearchForRequest(queryStr: query, n: 30);
  searchRequest.sendSignalToRust(); // GENERATED

  return (await SearchForResponse.rustSignalStream.first).message;
}
