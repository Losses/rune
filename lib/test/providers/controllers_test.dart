import 'dart:async';

import 'package:flutter_test/flutter_test.dart';

import '../../bindings/bindings.dart';
import '../../providers/controllers.dart';

AttachedControllerInfo controller(String fingerprint) => AttachedControllerInfo(
  id: 'id-$fingerprint',
  fingerprint: fingerprint,
  alias: fingerprint,
  connectedAt: Uint64.fromBigInt(BigInt.one),
);

void main() {
  test('loads the initial list and applies later controller updates', () async {
    final changes = StreamController<ControllersUpdated>();
    final provider = ControllersProvider(
      load: () async => [controller('device-a'), controller('device-a')],
      changes: changes.stream,
    );
    await Future<void>.delayed(Duration.zero);

    expect(provider.isControlling('device-a'), isTrue);
    expect(provider.isControlling('device-b'), isFalse);

    changes.add(ControllersUpdated(controllers: [controller('device-b')]));
    await Future<void>.delayed(Duration.zero);
    expect(provider.isControlling('device-a'), isFalse);
    expect(provider.isControlling('device-b'), isTrue);

    changes.add(const ControllersUpdated(controllers: []));
    await Future<void>.delayed(Duration.zero);
    expect(provider.isControlling('device-b'), isFalse);

    provider.dispose();
    await changes.close();
  });

  test('a live update wins over a delayed initial response', () async {
    final initial = Completer<List<AttachedControllerInfo>>();
    final changes = StreamController<ControllersUpdated>();
    final provider = ControllersProvider(
      load: () => initial.future,
      changes: changes.stream,
    );

    changes.add(ControllersUpdated(controllers: [controller('live-device')]));
    await Future<void>.delayed(Duration.zero);
    initial.complete([controller('stale-device')]);
    await Future<void>.delayed(Duration.zero);

    expect(provider.isControlling('live-device'), isTrue);
    expect(provider.isControlling('stale-device'), isFalse);

    provider.dispose();
    await changes.close();
  });
}
