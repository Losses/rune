import 'dart:async';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:rinf/rinf.dart';

import '../bindings/bindings.dart';
import '../utils/api/request_failure.dart';
import '../utils/api/search_for.dart';

void main() {
  test('interleaved replies complete their matching searches', () async {
    final responses =
        StreamController<RustSignalPack<SearchForResponse>>.broadcast();
    final failures = StreamController<RequestFailedResponse>.broadcast();
    final requests = <SearchForRequest>[];
    final searchA = searchForWithStreams(
      'ab',
      fields: const ['tracks'],
      responses: responses.stream,
      failures: failures.stream,
      sendRequest: requests.add,
    );
    final searchB = searchForWithStreams(
      'abc',
      fields: const ['albums'],
      responses: responses.stream,
      failures: failures.stream,
      sendRequest: requests.add,
    );

    responses.add(_response(requests[1].callId.toInt(), tracks: const [2]));
    responses.add(_response(requests[0].callId.toInt(), tracks: const [1]));

    expect((await searchA).tracks, [1]);
    expect((await searchB).tracks, [2]);
    await responses.close();
    await failures.close();
  });

  test('a failure only completes its matching search', () async {
    final responses =
        StreamController<RustSignalPack<SearchForResponse>>.broadcast();
    final failures = StreamController<RequestFailedResponse>.broadcast();
    final requests = <SearchForRequest>[];
    var searchACompleted = false;
    final searchA = searchForWithStreams(
      'a',
      fields: const ['tracks'],
      responses: responses.stream,
      failures: failures.stream,
      sendRequest: requests.add,
    );
    searchA.then<void>(
      (_) => searchACompleted = true,
      onError: (Object _) {
        searchACompleted = true;
      },
    );
    final searchB = searchForWithStreams(
      'b',
      fields: const ['tracks'],
      responses: responses.stream,
      failures: failures.stream,
      sendRequest: requests.add,
    );

    failures.add(_failure(requests[1].callId.toInt()));
    await expectLater(searchB, throwsA(isA<RequestFailure>()));
    await Future<void>.delayed(Duration.zero);
    expect(searchACompleted, isFalse);

    failures.add(_failure(requests[0].callId.toInt()));
    await expectLater(searchA, throwsA(isA<RequestFailure>()));
    await responses.close();
    await failures.close();
  });
}

RustSignalPack<SearchForResponse> _response(
  int callId, {
  required List<int> tracks,
}) => RustSignalPack(
  SearchForResponse(
    callId: Uint64.fromBigInt(BigInt.from(callId)),
    artists: const [],
    albums: const [],
    playlists: const [],
    tracks: tracks,
    genres: const [],
  ),
  Uint8List(0),
);

RequestFailedResponse _failure(int callId) => RequestFailedResponse(
  request: 'SearchForRequest',
  callId: Uint64.fromBigInt(BigInt.from(callId)),
  kind: RequestErrorKind.unavailable,
  detail: 'failed',
);
