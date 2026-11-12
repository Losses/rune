import 'dart:async';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import 'call_id.dart';

class RequestFailure implements Exception {
  final String request;
  final int? callId;
  final RequestErrorKind kind;
  final String detail;
  final RequestFailureCause? cause;

  const RequestFailure(
    this.request,
    this.kind,
    this.detail, [
    this.cause,
    this.callId,
  ]);

  @override
  String toString() => 'RequestFailure($request, $kind: $detail)';
}

Future<T> awaitResponse<T>(
  String request,
  Stream<T> responses, {
  Duration timeout = const Duration(seconds: 30),
  Stream<RequestFailedResponse>? failures,
  required int? Function(T value) responseCallId,
  required void Function(Uint64 callId) send,
}) async {
  final callId = nextCallId();
  final result = Completer<T>();
  final responseSubscription = responses.listen(
    (value) {
      if (responseCallId(value) != callId.toInt()) return;
      if (!result.isCompleted) result.complete(value);
    },
    onError: (Object error, StackTrace stack) {
      if (!result.isCompleted) result.completeError(error, stack);
    },
  );
  final failureSubscription =
      (failures ??
              RequestFailedResponse.rustSignalStream.map(
                (event) => event.message,
              ))
          .listen(
            (failure) {
              if (failure.callId == callId && !result.isCompleted) {
                result.completeError(
                  RequestFailure(
                    failure.request,
                    failure.kind,
                    failure.detail,
                    failure.cause,
                    callId.toInt(),
                  ),
                );
              }
            },
            onError: (Object error, StackTrace stack) {
              if (!result.isCompleted) result.completeError(error, stack);
            },
          );
  try {
    send(callId);
    return await result.future.timeout(timeout);
  } finally {
    await responseSubscription.cancel();
    await failureSubscription.cancel();
  }
}

String requestFailureMessage(AppLocalizations s, RequestFailure failure) =>
    switch (failure.cause) {
      RequestFailureCauseLibraryTaskAlreadyRunning(:final task) =>
        '${switch (task) {
          LibraryTaskKind.scan => s.scanning,
          LibraryTaskKind.analyze => s.analyzing,
          LibraryTaskKind.deduplicate => s.deduplicating,
        }}: ${s.requestUnavailable}',
      RequestFailureCauseReconnecting(:final alias) =>
        s.remoteLibraryReconnecting(alias),
      RequestFailureCauseStorageBusy() => s.requestStorageBusy,
      RequestFailureCauseCertificateMismatch() => s.requestCertificateMismatch,
      RequestFailureCausePublicKeyMismatch() => s.requestPublicKeyMismatch,
      RequestFailureCauseRegistrationConflict() =>
        s.requestRegistrationConflict,
      RequestFailureCausePeerNeedsUpdate(:final updateSide, :final peerAlias) =>
        switch (updateSide) {
          ProtocolUpdateSide.peer => s.requestPeerNeedsUpdatePeer(
            peerAlias ?? s.requestOtherDeviceName,
          ),
          ProtocolUpdateSide.thisDevice => s.requestPeerNeedsUpdateThisDevice(
            peerAlias ?? s.requestOtherDeviceName,
          ),
        },
      _ => switch (failure.kind) {
        RequestErrorKind.notFound => s.requestItemMissing,
        RequestErrorKind.invalidInput => s.requestInvalid,
        RequestErrorKind.unavailable => s.requestUnavailable,
        RequestErrorKind.permissionDenied => s.requestPermissionDenied,
        RequestErrorKind.conflict => s.requestConflict,
        RequestErrorKind.peerNeedsUpdate => s.requestPeerNeedsUpdateFallback,
        RequestErrorKind.internal => s.unknownError,
      },
    };

String requestErrorText(AppLocalizations s, Object error) =>
    error is RequestFailure ? requestFailureMessage(s, error) : 'Error: $error';
