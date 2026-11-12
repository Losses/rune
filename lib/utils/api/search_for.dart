import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:rinf/rinf.dart';

import '../../bindings/bindings.dart';
import 'request_failure.dart';

Future<SearchForResponse> searchFor(
  String query, {
  List<SearchField> fields = const [],
}) {
  return _searchFor(
    query,
    fields,
    responses: SearchForResponse.rustSignalStream,
    sendRequest: (request) => request.sendSignalToRust(),
  );
}

@visibleForTesting
Future<SearchForResponse> searchForWithStreams(
  String query, {
  required List<SearchField> fields,
  required Stream<RustSignalPack<SearchForResponse>> responses,
  required Stream<RequestFailedResponse> failures,
  required void Function(SearchForRequest) sendRequest,
  Duration timeout = const Duration(seconds: 10),
}) => _searchFor(
  query,
  fields,
  responses: responses,
  failures: failures,
  sendRequest: sendRequest,
  timeout: timeout,
);

Future<SearchForResponse> _searchFor(
  String query,
  List<SearchField> fields, {
  required Stream<RustSignalPack<SearchForResponse>> responses,
  Stream<RequestFailedResponse>? failures,
  required void Function(SearchForRequest) sendRequest,
  Duration timeout = const Duration(seconds: 10),
}) async {
  if (!query.endsWith('*')) query = '$query*';
  final response = await awaitResponse(
    'SearchForRequest',
    responses,
    timeout: timeout,
    failures: failures,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => sendRequest(
      SearchForRequest(callId: callId, queryStr: query, n: 30, fields: fields),
    ),
  );
  return response.message;
}
