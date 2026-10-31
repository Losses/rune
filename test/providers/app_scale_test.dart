import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:rune/providers/app_scale.dart';

void main() {
  test('loads only supported integer settings', () async {
    for (final value in [null, '125', 125.0, 101, double.nan]) {
      final scale = AppScaleProvider(
        read: () async => value,
        write: (_) async {},
      );
      await scale.initialize();
      expect(scale.percentage, 100);
      scale.dispose();
    }
    for (final value in AppScaleProvider.supportedPercentages) {
      final scale = AppScaleProvider(
        read: () async => value,
        write: (_) async {},
      );
      await scale.initialize();
      expect(scale.scaleFactor, value / 100);
      scale.dispose();
    }
  });

  test('writes in order and does not publish a failed write', () async {
    final writes = <int>[];
    final scale = AppScaleProvider(
      read: () async => null,
      write: (value) async {
        if (value == 150) throw StateError('failed');
        writes.add(value);
      },
    );
    final first = scale.setPercentage(125);
    final second = scale.setPercentage(200);
    await Future.wait([first, second]);
    expect(writes, [125, 200]);
    expect(scale.percentage, 200);
    await expectLater(scale.setPercentage(150), throwsStateError);
    expect(scale.percentage, 200);
    await scale.setPercentage(100);
    expect(scale.percentage, 100);
    scale.dispose();
  });
  test('late initialization cannot overwrite a newer choice', () async {
    final read = Completer<Object?>();
    final scale = AppScaleProvider(
      read: () => read.future,
      write: (_) async {},
    );
    final initialization = scale.initialize();
    await scale.setPercentage(150);
    read.complete(75);
    await initialization;
    expect(scale.percentage, 150);
    scale.dispose();
  });

  test('pending read and write may finish after disposal', () async {
    final read = Completer<Object?>();
    final write = Completer<void>();
    final scale = AppScaleProvider(
      read: () => read.future,
      write: (_) => write.future,
    );
    final initialization = scale.initialize();
    final operation = scale.setPercentage(125);
    await Future<void>.delayed(Duration.zero);
    scale.dispose();
    read.complete(75);
    write.complete();
    await Future.wait([initialization, operation]);
  });
}
