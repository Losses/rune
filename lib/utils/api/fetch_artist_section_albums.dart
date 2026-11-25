import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:rinf/rinf.dart';

import '../../bindings/bindings.dart';
import 'request_failure.dart';

/// A page of the albums of a section of an artist's page that opens as albums:
/// their ids in the groups the library gives them, in order. A group the page
/// before ended with goes on in this page's first group of the same title. A
/// page of fewer albums than [pageSize] is the last.
///
/// Only a library that gives such sections answers.
Future<List<ArtistSectionAlbumGroup>> fetchArtistSectionAlbums(
  List<MixQuery> queries,
  int cursor,
  int pageSize,
) {
  return _fetchArtistSectionAlbums(
    queries,
    cursor,
    pageSize,
    responses: FetchArtistSectionAlbumsResponse.rustSignalStream,
    sendRequest: (request) => request.sendSignalToRust(),
  );
}

@visibleForTesting
Future<List<ArtistSectionAlbumGroup>> fetchArtistSectionAlbumsWithStreams(
  List<MixQuery> queries,
  int cursor,
  int pageSize, {
  required Stream<RustSignalPack<FetchArtistSectionAlbumsResponse>> responses,
  required Stream<RequestFailedResponse> failures,
  required void Function(FetchArtistSectionAlbumsRequest) sendRequest,
  Duration timeout = const Duration(seconds: 30),
}) => _fetchArtistSectionAlbums(
  queries,
  cursor,
  pageSize,
  responses: responses,
  failures: failures,
  sendRequest: sendRequest,
  timeout: timeout,
);

Future<List<ArtistSectionAlbumGroup>> _fetchArtistSectionAlbums(
  List<MixQuery> queries,
  int cursor,
  int pageSize, {
  required Stream<RustSignalPack<FetchArtistSectionAlbumsResponse>> responses,
  Stream<RequestFailedResponse>? failures,
  required void Function(FetchArtistSectionAlbumsRequest) sendRequest,
  Duration timeout = const Duration(seconds: 30),
}) async {
  final response = await awaitResponse(
    'FetchArtistSectionAlbumsRequest',
    responses,
    timeout: timeout,
    failures: failures,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => sendRequest(
      FetchArtistSectionAlbumsRequest(
        callId: callId,
        queries: queries,
        cursor: cursor,
        pageSize: pageSize,
      ),
    ),
  );
  return response.message.groups;
}
