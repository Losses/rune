import 'dart:async';

import 'package:flutter_test/flutter_test.dart';

import '../../bindings/bindings.dart';
import '../../utils/api/controllers.dart';

void main() {
  test(
    'detaching a controller completes on the reply with its call id',
    () async {
      final responses = StreamController<DetachControllerResponse>();
      final sent = <DetachControllerRequest>[];
      var done = false;
      final result = detachController(
        'controller-1',
        responses: responses.stream,
        send: sent.add,
      ).then((_) => done = true);

      expect(sent.single.id, 'controller-1');
      final callId = sent.single.callId!;

      responses.add(
        DetachControllerResponse(
          callId: Uint64.fromBigInt(BigInt.from(callId.toInt() + 1)),
        ),
      );
      await Future<void>.delayed(Duration.zero);
      expect(done, isFalse, reason: 'a reply to another call must not count');

      responses.add(DetachControllerResponse(callId: callId));
      await result;
      expect(done, isTrue);
      expect(responses.hasListener, isFalse);
      await responses.close();
    },
  );
}
