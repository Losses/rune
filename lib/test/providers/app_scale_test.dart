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

  group('text size following', () {
    const textScales = <String, double>{
      'xS': 14 / 17,
      'S': 15 / 17,
      'M': 16 / 17,
      'L': 1.0,
      'xL': 19 / 17,
      'xxL': 21 / 17,
      'xxxL': 23 / 17,
      'AX1': 28 / 17,
      'AX2': 33 / 17,
      'AX3': 40 / 17,
      'AX4': 47 / 17,
      'AX5': 53 / 17,
    };
    const expected = <int, Map<String, double>>{
      3: {
        'L': 1.0,
        'xL': 1.0,
        'xxL': 4 / 3,
        'xxxL': 4 / 3,
        'AX1': 5 / 3,
        'AX2': 2.0,
        'AX3': 7 / 3,
        'AX4': 8 / 3,
        'AX5': 3.0,
      },
      2: {
        'L': 1.0,
        'xL': 1.0,
        'xxL': 1.0,
        'xxxL': 1.5,
        'AX1': 1.5,
        'AX2': 2.0,
        'AX3': 2.5,
        'AX4': 3.0,
        'AX5': 3.0,
      },
    };

    test('maps every iOS text size to whole device pixels', () {
      for (final MapEntry(key: dpr, value: sizes) in expected.entries) {
        for (final MapEntry(key: name, value: factor) in sizes.entries) {
          expect(
            textSizeScaleFactor(textScales[name]!, dpr.toDouble()),
            closeTo(factor, 1e-9),
            reason: '$name at DPR $dpr',
          );
        }
      }
    });

    test('keeps the invariants for every size and DPR', () {
      for (final dpr in <double>[2, 3]) {
        for (final MapEntry(key: name, value: t) in textScales.entries) {
          final factor = textSizeScaleFactor(t, dpr);
          expect(factor * dpr, closeTo((factor * dpr).round(), 1e-9));
          if (t <= 1) {
            expect(factor, 1.0, reason: '$name at DPR $dpr');
          } else {
            // The nearest whole-pixel step, never below 1.
            expect((factor - t).abs(), lessThanOrEqualTo(0.5 / dpr));
            expect(factor, greaterThanOrEqualTo(1));
          }
        }
      }
    });

    test('ignores an invalid device pixel ratio', () {
      expect(textSizeScaleFactor(2, 0), 1.0);
      expect(textSizeScaleFactor(2, -1), 1.0);
    });

    test('does not follow the text size unless asked to', () {
      expect(AppScaleProvider().supportsFollowTextSize, isFalse);
    });

    test('defaults to following only where supported', () async {
      final supported = AppScaleProvider(
        read: () async => null,
        write: (_) async {},
        supportsFollowTextSize: true,
      );
      expect(supported.defaultPercentage, AppScaleProvider.followTextSize);
      await supported.initialize();
      expect(supported.followingTextSize, isTrue);
      expect(supported.percentage, AppScaleProvider.followTextSize);
      supported.dispose();
      final unsupported = AppScaleProvider(
        read: () async => null,
        write: (_) async {},
        supportsFollowTextSize: false,
      );
      expect(unsupported.defaultPercentage, 100);
      await unsupported.initialize();
      expect(unsupported.followingTextSize, isFalse);
      expect(unsupported.percentage, 100);
      unsupported.dispose();
    });

    test('a stored follow choice needs platform support', () async {
      final unsupported = AppScaleProvider(
        read: () async => AppScaleProvider.followTextSize,
        write: (_) async {},
        supportsFollowTextSize: false,
      );
      await unsupported.initialize();
      expect(unsupported.percentage, 100);
      expect(
        () => unsupported.setPercentage(AppScaleProvider.followTextSize),
        throwsArgumentError,
      );
      unsupported.dispose();
      final supported = AppScaleProvider(
        read: () async => 150,
        write: (_) async {},
        supportsFollowTextSize: true,
      );
      await supported.initialize();
      expect(supported.percentage, 150);
      expect(supported.scaleFactor, 1.5);
      supported.dispose();
    });

    testWidgets('follows live text size and pixel ratio changes', (
      tester,
    ) async {
      tester.view.devicePixelRatio = 3;
      tester.platformDispatcher.textScaleFactorTestValue = 1.0;
      addTearDown(tester.view.resetDevicePixelRatio);
      addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
      final saved = <int>[];
      final scale = AppScaleProvider(
        read: () async => null,
        write: (value) async => saved.add(value),
        supportsFollowTextSize: true,
      );
      await scale.initialize();
      var notified = 0;
      scale.addListener(() => notified++);
      expect(scale.scaleFactor, 1.0);
      tester.platformDispatcher.textScaleFactorTestValue = 33 / 17;
      await tester.pump();
      expect(scale.scaleFactor, closeTo(2.0, 1e-9));
      expect(notified, 1);
      tester.view.devicePixelRatio = 2;
      await tester.pump();
      expect(scale.scaleFactor, closeTo(2.0, 1e-9));
      tester.platformDispatcher.textScaleFactorTestValue = 40 / 17;
      await tester.pump();
      expect(scale.scaleFactor, closeTo(2.5, 1e-9));
      final changes = notified;
      await scale.setPercentage(100);
      expect(saved, [100]);
      expect(scale.scaleFactor, 1.0);
      expect(notified, changes + 1);
      tester.platformDispatcher.textScaleFactorTestValue = 53 / 17;
      await tester.pump();
      expect(scale.scaleFactor, 1.0);
      expect(notified, changes + 1);
      await scale.setPercentage(AppScaleProvider.followTextSize);
      expect(scale.scaleFactor, closeTo(3.0, 1e-9));
      scale.dispose();
    });
  });
}
