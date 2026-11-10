import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:rinf/rinf.dart';

import '../../bindings/bindings.dart';
import 'request_failure.dart';

// These ids match search replies to calls; they do not route requests.
var _nextSearchCallId = 0;

Future<SearchForResponse> searchFor(
  String query, {
  List<String> fields = const [],
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
  required List<String> fields,
  required Stream<RustSignalPack<SearchForResponse>> responses,
  required Stream<RequestFailedResponse> failures,
  required void Function(SearchForRequest) sendRequest,
}) => _searchFor(
  query,
  fields,
  responses: responses,
  failures: failures,
  sendRequest: sendRequest,
);

Future<SearchForResponse> _searchFor(
  String query,
  List<String> fields, {
  required Stream<RustSignalPack<SearchForResponse>> responses,
  Stream<RequestFailedResponse>? failures,
  required void Function(SearchForRequest) sendRequest,
}) async {
  if (!query.endsWith('*')) query = '$query*';
  final callId = ++_nextSearchCallId;
  final response = await awaitResponse(
    'SearchForRequest',
    responses,
    timeout: const Duration(seconds: 10),
    failures: failures,
    callId: callId,
    responseCallId: (signal) => signal.message.callId?.toInt(),
    send: () => sendRequest(
      SearchForRequest(
        callId: Uint64.fromBigInt(BigInt.from(callId)),
        queryStr: query,
        n: 30,
        fields: fields,
      ),
    ),
  );
  return response.message;
}
