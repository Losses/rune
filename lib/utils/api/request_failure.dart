import 'dart:async';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';

class RequestFailure implements Exception {
  final String request;
  final RequestErrorKind kind;
  final String detail;
  final RequestFailureCause? cause;

  const RequestFailure(this.request, this.kind, this.detail, [this.cause]);

  @override
  String toString() => 'RequestFailure($request, $kind: $detail)';
}

Future<T> awaitResponse<T>(
  String request,
  Stream<T> responses, {
  Duration timeout = const Duration(seconds: 30),
  Stream<RequestFailedResponse>? failures,
  int? callId,
  int? Function(T value)? responseCallId,
  void Function()? send,
}) async {
  assert(
    callId == null || responseCallId != null,
    'callId needs responseCallId to read the id from a response',
  );
  final result = Completer<T>();
  final responseSubscription = responses.listen(
    (value) {
      if (callId != null && responseCallId?.call(value) != callId) return;
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
              if (failure.request == request &&
                  (callId == null || failure.callId?.toInt() == callId) &&
                  !result.isCompleted) {
                result.completeError(
                  RequestFailure(
                    failure.request,
                    failure.kind,
                    failure.detail,
                    failure.cause,
                  ),
                );
              }
            },
            onError: (Object error, StackTrace stack) {
              if (!result.isCompleted) result.completeError(error, stack);
            },
          );
  try {
    send?.call();
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
      RequestFailureCauseRegistrationConflict() => s.requestRegistrationConflict,
      RequestFailureCausePeerNeedsUpdate(
        :final updateSide,
        :final peerAlias,
      ) => switch (updateSide) {
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
