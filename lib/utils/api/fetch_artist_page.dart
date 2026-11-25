import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:rinf/rinf.dart';

import '../../bindings/bindings.dart';
import 'request_failure.dart';

/// The page of an artist: its release sections and the queries of its tracks.
///
/// Only a library whose capability `fetchArtistPageRequest` is available answers.
/// [language] is the language the app shows, as a BCP 47 tag: the library
/// names the sections in it where it can.
Future<FetchArtistPageResponse> fetchArtistPage(int artistId, String language) {
  return _fetchArtistPage(
    artistId,
    language,
    responses: FetchArtistPageResponse.rustSignalStream,
    sendRequest: (request) => request.sendSignalToRust(),
  );
}

@visibleForTesting
Future<FetchArtistPageResponse> fetchArtistPageWithStreams(
  int artistId,
  String language, {
  required Stream<RustSignalPack<FetchArtistPageResponse>> responses,
  required Stream<RequestFailedResponse> failures,
  required void Function(FetchArtistPageRequest) sendRequest,
  Duration timeout = const Duration(seconds: 30),
}) => _fetchArtistPage(
  artistId,
  language,
  responses: responses,
  failures: failures,
  sendRequest: sendRequest,
  timeout: timeout,
);

Future<FetchArtistPageResponse> _fetchArtistPage(
  int artistId,
  String language, {
  required Stream<RustSignalPack<FetchArtistPageResponse>> responses,
  Stream<RequestFailedResponse>? failures,
  required void Function(FetchArtistPageRequest) sendRequest,
  Duration timeout = const Duration(seconds: 30),
}) async {
  final response = await awaitResponse(
    'FetchArtistPageRequest',
    responses,
    timeout: timeout,
    failures: failures,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => sendRequest(
      FetchArtistPageRequest(
        callId: callId,
        artistId: artistId,
        language: language,
      ),
    ),
  );
  return response.message;
}
