import 'package:flutter/foundation.dart';
import 'package:rinf/rinf.dart';

import '../../bindings/bindings.dart';
import 'request_failure.dart';

Future<void> removeItemFromPlaylist(int playlistId, String occurrenceId) async {
  await _removeItemFromPlaylist(
    playlistId,
    occurrenceId,
    responses: RemoveItemFromPlaylistResponse.rustSignalStream,
    sendRequest: (request) => request.sendSignalToRust(),
  );
}

@visibleForTesting
Future<void> removeItemFromPlaylistWithStreams(
  int playlistId,
  String occurrenceId, {
  required Stream<RustSignalPack<RemoveItemFromPlaylistResponse>> responses,
  required Stream<RequestFailedResponse> failures,
  required void Function(RemoveItemFromPlaylistRequest) sendRequest,
}) => _removeItemFromPlaylist(
  playlistId,
  occurrenceId,
  responses: responses,
  failures: failures,
  sendRequest: sendRequest,
);

Future<void> _removeItemFromPlaylist(
  int playlistId,
  String occurrenceId, {
  required Stream<RustSignalPack<RemoveItemFromPlaylistResponse>> responses,
  Stream<RequestFailedResponse>? failures,
  required void Function(RemoveItemFromPlaylistRequest) sendRequest,
}) async {
  await awaitResponse(
    'RemoveItemFromPlaylistRequest',
    responses,
    failures: failures,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => sendRequest(
      RemoveItemFromPlaylistRequest(
        playlistId: playlistId,
        occurrenceId: occurrenceId,
        callId: callId,
      ),
    ),
  );
}
