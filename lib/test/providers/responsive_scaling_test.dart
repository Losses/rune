import 'dart:async';
import 'dart:ui' show Size;

import 'package:flutter_test/flutter_test.dart';
import 'package:rune/providers/app_scale.dart';
import 'package:rune/providers/responsive_providers.dart';

void main() {
  testWidgets(
    'responsive metrics use scale while window persistence uses raw size',
    (tester) async {
      tester.view.devicePixelRatio = 2;
      tester.view.physicalSize = const Size(2400, 1200);
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final saved = <String>[];
      final scale = AppScaleProvider(
        read: () async => null,
        write: (_) async {},
      );
      final sizes = ScreenSizeProvider.withWriter(
        scale,
        (value) async => saved.add(value),
      );
      final responsive = ResponsiveProvider(sizes, () async => null);
      await tester.pump(const Duration(seconds: 2));
      expect(sizes.rawWindowSize, const Size(1200, 600));
      expect(saved, ['1200.0,600.0']);
      await scale.setPercentage(200);
      expect(sizes.screenSize, const Size(600, 300));
      await tester.pump(const Duration(seconds: 2));
      expect(saved, hasLength(1));
      tester.view.physicalSize = const Size(242, 1200);
      await tester.pump();
      expect(sizes.screenSize.width, 60.5);
      await scale.setPercentage(100);
      expect(sizes.screenSize.width, 121);
      expect(responsive.equalTo(DeviceType.zune), isTrue);
      tester.view.physicalSize = const Size(241, 1200);
      await tester.pump();
      expect(sizes.screenSize.width, 120.5);
      expect(responsive.equalTo(DeviceType.dock), isTrue);
      tester.view.physicalSize = Size.zero;
      await tester.pump();
      await tester.pump(const Duration(seconds: 2));
      expect(saved, hasLength(1));
      responsive.dispose();
      sizes.dispose();
      scale.dispose();
    },
  );

  testWidgets(
    'ignores invalid force layout and safely finishes after dispose',
    (tester) async {
      final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
      final pending = Completer<String?>();
      final responsive = ResponsiveProvider(sizes, () => pending.future);
      responsive.dispose();
      pending.complete('invalid');
      await tester.pump();
      expect(tester.takeException(), isNull);
      sizes.dispose();
    },
  );

  testWidgets('following the text size moves an iPhone from phone to zune', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 3;
    tester.view.physicalSize = const Size(1206, 2622);
    tester.platformDispatcher.textScaleFactorTestValue = 1.0;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
    final scale = AppScaleProvider(
      read: () async => null,
      write: (_) async {},
      supportsFollowTextSize: true,
    );
    await scale.initialize();
    final sizes = ScreenSizeProvider.withWriter(scale, (_) async {});
    final responsive = ResponsiveProvider(sizes, () async => null);
    expect(sizes.screenSize, const Size(402, 874));
    expect(responsive.equalTo(DeviceType.phone), isTrue);
    // One size larger is nearest to 100%, so the layout stays.
    tester.platformDispatcher.textScaleFactorTestValue = 19 / 17;
    await tester.pump();
    expect(responsive.equalTo(DeviceType.phone), isTrue);
    tester.platformDispatcher.textScaleFactorTestValue = 21 / 17;
    await tester.pump();
    expect(responsive.equalTo(DeviceType.zune), isTrue);
    await tester.pump(const Duration(seconds: 2));
    responsive.dispose();
    sizes.dispose();
    scale.dispose();
  });
}
