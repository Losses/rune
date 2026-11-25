import 'dart:async';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:rinf/rinf.dart';

import '../bindings/bindings.dart';
import '../utils/api/fetch_artist_page.dart';
import '../utils/api/request_failure.dart';

void main() {
  test(
    'the request names the artist and the language, and the reply of its call '
    'completes it',
    () async {
      final responses =
          StreamController<RustSignalPack<FetchArtistPageResponse>>.broadcast();
      final failures = StreamController<RequestFailedResponse>.broadcast();
      final requests = <FetchArtistPageRequest>[];
      final page = fetchArtistPageWithStreams(
        7,
        'zh-Hans',
        responses: responses.stream,
        failures: failures.stream,
        sendRequest: requests.add,
      );

      expect(requests.single.artistId, 7);
      expect(requests.single.language, 'zh-Hans');
      // A reply to another call is not this page.
      responses.add(
        _response(requests.single.callId!.toInt() - 1, artistId: 8),
      );
      responses.add(_response(requests.single.callId!.toInt(), artistId: 7));

      final response = await page;
      expect(response.artistId, 7);
      expect(response.sections.single.kind, ArtistSectionKind.eps);
      expect(
        response.sections.single.queries.single.operator,
        'lib::artist_section',
      );
      expect(response.sections.single.queries.single.parameter, '7:eps');
      expect(response.tracks.single.operator, 'lib::artist_tracks');
      await responses.close();
      await failures.close();
    },
  );

  test('a failure only completes its matching page', () async {
    final responses =
        StreamController<RustSignalPack<FetchArtistPageResponse>>.broadcast();
    final failures = StreamController<RequestFailedResponse>.broadcast();
    final requests = <FetchArtistPageRequest>[];
    var firstCompleted = false;
    final first = fetchArtistPageWithStreams(
      1,
      'en',
      responses: responses.stream,
      failures: failures.stream,
      sendRequest: requests.add,
    );
    first.then<void>(
      (_) => firstCompleted = true,
      onError: (Object _) {
        firstCompleted = true;
      },
    );
    final second = fetchArtistPageWithStreams(
      2,
      'en',
      responses: responses.stream,
      failures: failures.stream,
      sendRequest: requests.add,
    );

    failures.add(_failure(requests[1].callId!.toInt()));
    await expectLater(second, throwsA(isA<RequestFailure>()));
    await Future<void>.delayed(Duration.zero);
    expect(firstCompleted, isFalse);

    failures.add(_failure(requests[0].callId!.toInt()));
    await expectLater(first, throwsA(isA<RequestFailure>()));
    await responses.close();
    await failures.close();
  });
}

RustSignalPack<FetchArtistPageResponse> _response(
  int callId, {
  required int artistId,
}) => RustSignalPack(
  FetchArtistPageResponse(
    callId: Uint64.fromBigInt(BigInt.from(callId)),
    artistId: artistId,
    sections: [
      ArtistSection(
        kind: ArtistSectionKind.eps,
        title: '',
        opens: ArtistSectionOpens.songs,
        queries: [
          MixQuery(operator: 'lib::artist_section', parameter: '$artistId:eps'),
        ],
        covers: const [],
        count: 2,
      ),
    ],
    tracks: [MixQuery(operator: 'lib::artist_tracks', parameter: '$artistId')],
  ),
  Uint8List(0),
);

RequestFailedResponse _failure(int callId) => RequestFailedResponse(
  request: 'FetchArtistPageRequest',
  callId: Uint64.fromBigInt(BigInt.from(callId)),
  kind: RequestErrorKind.unavailable,
  detail: 'failed',
);
