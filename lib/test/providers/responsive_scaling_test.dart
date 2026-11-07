import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/providers/app_scale.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/widgets/app_viewport.dart';

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

  group('following a large text size', () {
    // An iPhone with the system text size [textScale], in points, as the app
    // starts with it. Gives the layout and the size a 17 pt font ends up at.
    Future<(DeviceType, double)> open(
      WidgetTester tester, {
      required Size size,
      required double devicePixelRatio,
      required EdgeInsets viewPadding,
      required double textScale,
    }) async {
      tester.view.devicePixelRatio = devicePixelRatio;
      tester.view.physicalSize = size * devicePixelRatio;
      final padding = viewPadding * devicePixelRatio;
      tester.view.viewPadding = FakeViewPadding(
        left: padding.left,
        top: padding.top,
        right: padding.right,
        bottom: padding.bottom,
      );
      tester.platformDispatcher.textScaleFactorTestValue = textScale;
      addTearDown(tester.view.reset);
      addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
      final scale = AppScaleProvider(
        read: () async => null,
        write: (_) async {},
        supportsFollowTextSize: true,
      );
      await scale.initialize();
      final sizes = ScreenSizeProvider.withWriter(scale, (_) async {});
      final responsive = ResponsiveProvider(sizes, () async => null);
      addTearDown(scale.dispose);
      addTearDown(sizes.dispose);
      addTearDown(responsive.dispose);

      late TextScaler text;
      await tester.pumpWidget(
        ChangeNotifierProvider.value(
          value: scale,
          child: AppViewport(
            child: Builder(
              builder: (context) {
                text = MediaQuery.textScalerOf(context);
                return const SizedBox.expand();
              },
            ),
          ),
        ),
      );
      await tester.pump(const Duration(seconds: 2));
      return (
        responsive.currentDeviceType as DeviceType,
        text.scale(17) * scale.scaleFactor,
      );
    }

    // A phone's layouts: zune and the upright ones wider than it.
    final phoneLayouts = [
      DeviceType.zune,
      DeviceType.phone,
      DeviceType.mobile,
      DeviceType.tablet,
      DeviceType.desktop,
    ];

    for (final (name, textScale) in [('xxL', 21 / 17), ('AX4', 47 / 17)]) {
      testWidgets('keeps a phone on its side in its layout at $name', (
        tester,
      ) async {
        final (layout, font) = await open(
          tester,
          size: const Size(852, 393),
          devicePixelRatio: 3,
          viewPadding: const EdgeInsets.only(left: 59, right: 59, bottom: 21),
          textScale: textScale,
        );
        expect(layout, isIn(phoneLayouts));
        expect(font, closeTo(17 * textScale, 1e-9));
      }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));
    }

    testWidgets('keeps an iPhone SE upright in zune at AX5', (tester) async {
      final (layout, font) = await open(
        tester,
        size: const Size(375, 667),
        devicePixelRatio: 2,
        viewPadding: const EdgeInsets.only(top: 20),
        textScale: 53 / 17,
      );
      expect(layout, DeviceType.zune);
      expect(font, closeTo(53, 1e-9));
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

    testWidgets('a turn holds the zoom at the floor and lets it go again', (
      tester,
    ) async {
      tester.view.devicePixelRatio = 3;
      tester.view.physicalSize = const Size(1179, 2556);
      tester.view.viewPadding = const FakeViewPadding(top: 177, bottom: 102);
      tester.platformDispatcher.textScaleFactorTestValue = 47 / 17;
      addTearDown(tester.view.reset);
      addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
      final scale = AppScaleProvider(
        read: () async => null,
        write: (_) async {},
        supportsFollowTextSize: true,
      );
      addTearDown(scale.dispose);
      await scale.initialize();
      expect(scale.scaleFactor, closeTo(8 / 3, 1e-9));

      tester.view.physicalSize = const Size(2556, 1179);
      tester.view.viewPadding = const FakeViewPadding(
        left: 177,
        right: 177,
        bottom: 63,
      );
      await tester.pump();
      expect(scale.scaleFactor, 1.0);

      tester.view.physicalSize = const Size(1179, 2556);
      tester.view.viewPadding = const FakeViewPadding(top: 177, bottom: 102);
      await tester.pump();
      expect(scale.scaleFactor, closeTo(8 / 3, 1e-9));
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));
  });
}
