import 'dart:async';

import 'package:flutter_test/flutter_test.dart';

import '../bindings/bindings.dart';
import '../providers/volume.dart';

void main() {
  test(
    'a user volume change wins over the pending stored volume read',
    () async {
      final storedVolume = Completer<double?>();
      final sent = <VolumeRequest>[];
      final provider = VolumeProvider(
        readStoredVolume: () => storedVolume.future,
        writeStoredVolume: (_) async {},
        sendVolumeRequest: sent.add,
      );
      addTearDown(provider.dispose);

      provider.updateVolume(0.35);
      storedVolume.complete(0.9);
      await Future<void>.delayed(const Duration(milliseconds: 250));

      expect(provider.volume, 0.35);
      expect(sent, hasLength(1));
      expect(sent.last.volume, 0.35);
    },
  );
}
