import 'dart:async';

import '../../bindings/bindings.dart';
import 'request_failure.dart';

Future<List<RendererCandidate>> listRenderers(List<String> libraryIds) async {
  final response = await awaitResponse(
    'ListRenderersRequest',
    ListRenderersResponse.rustSignalStream,
    responseCallId: (response) => response.message.callId?.toInt(),
    send: (callId) => ListRenderersRequest(
      libraryIds: libraryIds,
      callId: callId,
    ).sendSignalToRust(),
  );
  return response.message.renderers;
}

class HandoffOperation {
  const HandoffOperation(this.result, this.cancel);

  final Future<void> result;
  final void Function() cancel;
}

/// Moves the queue and position to the renderer [target], or back to this
/// device when [target] is null.
HandoffOperation handoffRenderer(String? target) {
  String? ticket;
  var cancelled = false;

  void cancel() {
    if (cancelled) return;
    cancelled = true;
    final current = ticket;
    if (current != null) {
      CancelHandoffRequest(ticket: current).sendSignalToRust();
    }
  }

  Future<void> run() async {
    try {
      final prepared = await awaitResponse(
        'PrepareHandoffRequest',
        PrepareHandoffResponse.rustSignalStream,
        responseCallId: (response) => response.message.callId?.toInt(),
        send: (callId) =>
            PrepareHandoffRequest(callId: callId).sendSignalToRust(),
      );
      final preparedTicket = prepared.message.ticket;
      ticket = preparedTicket;
      if (cancelled) {
        CancelHandoffRequest(ticket: preparedTicket).sendSignalToRust();
        return;
      }
      await awaitResponse(
        'HandoffRequest',
        HandoffResponse.rustSignalStream,
        responseCallId: (response) => response.message.callId?.toInt(),
        send: (callId) => HandoffRequest(
          target: target,
          ticket: preparedTicket,
          callId: callId,
        ).sendSignalToRust(),
      );
    } on TimeoutException {
      cancel();
      throw const RequestFailure(
        'HandoffRequest',
        RequestErrorKind.unavailable,
        'The handoff timed out.',
      );
    }
  }

  return HandoffOperation(run(), cancel);
}
