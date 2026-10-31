import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/providers/app_scale.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/widgets/app_viewport.dart';

void main() {
  testWidgets(
    'scales layout, metrics and hit testing while retaining navigator and input',
    (tester) async {
      final scale = AppScaleProvider(
        read: () async => null,
        write: (_) async {},
      );
      final navigator = GlobalKey<NavigatorState>();
      final field = GlobalKey();
      final controller = TextEditingController();
      final focus = FocusNode();
      var taps = 0;
      late MediaQueryData media;
      await tester.pumpWidget(
        ChangeNotifierProvider.value(
          value: scale,
          child: MaterialApp(
            navigatorKey: navigator,
            builder: (context, child) => AppViewport(child: child!),
            home: Builder(
              builder: (context) {
                media = MediaQuery.of(context);
                return Scaffold(
                  body: Column(
                    children: [
                      GestureDetector(
                        onTap: () => taps++,
                        child: const SizedBox(
                          width: 100,
                          height: 40,
                          child: Text('tap'),
                        ),
                      ),
                      SizedBox(
                        width: 200,
                        child: TextField(
                          key: field,
                          controller: controller,
                          focusNode: focus,
                        ),
                      ),
                    ],
                  ),
                );
              },
            ),
          ),
        ),
      );
      await tester.tap(find.text('tap'));
      await tester.tap(find.byKey(field));
      await tester.enterText(find.byKey(field), 'hello');
      controller.value = const TextEditingValue(
        text: 'hello',
        selection: TextSelection.collapsed(offset: 5),
        composing: TextRange(start: 0, end: 5),
      );
      tester.view.physicalSize = const Size(1600, 1200);
      tester.view.devicePixelRatio = 2;
      tester.view.viewInsets = const FakeViewPadding(bottom: 200);
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      addTearDown(tester.view.resetViewInsets);
      await tester.pump();
      final nav = navigator.currentState;
      final editable = tester.state<EditableTextState>(
        find.byType(EditableText),
      );
      await scale.setPercentage(200);
      await tester.pumpAndSettle();
      expect(media.size, const Size(400, 300));
      expect(media.devicePixelRatio, 4);
      expect(navigator.currentState, same(nav));
      expect(
        tester.state<EditableTextState>(find.byType(EditableText)),
        same(editable),
      );
      expect(focus.hasFocus, isTrue);
      expect(controller.text, 'hello');
      expect(controller.value.composing, const TextRange(start: 0, end: 5));
      expect(media.viewInsets.bottom, 50);
      tester.view.devicePixelRatio = 4;
      tester.view.physicalSize = const Size(2400, 1600);
      tester.view.viewInsets = const FakeViewPadding(bottom: 400);
      await tester.pump();
      expect(media.size, const Size(300, 200));
      expect(media.viewInsets.bottom, 50);
      expect(navigator.currentState, same(nav));
      expect(
        tester.state<EditableTextState>(find.byType(EditableText)),
        same(editable),
      );
      expect(focus.hasFocus, isTrue);
      expect(controller.value.composing, const TextRange(start: 0, end: 5));
      final inputGeometry = tester.testTextInput.log.lastWhere(
        (call) => call.method == 'TextInput.setEditableSizeAndTransform',
      );
      final transform = (inputGeometry.arguments as Map)['transform'] as List;
      expect(transform[0], 2.0);
      expect(transform[5], 2.0);
      await tester.tap(find.text('tap'));
      expect(taps, 2);
      await tester.pumpWidget(const SizedBox());
      focus.dispose();
      controller.dispose();
      scale.dispose();
    },
  );
  testWidgets('scales geometric metrics and follows window DPR and zero size', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 2;
    tester.view.physicalSize = const Size(1600, 1200);
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final scale = AppScaleProvider(read: () async => 200, write: (_) async {});
    await scale.initialize();
    final screen = ScreenSizeProvider.withWriter(scale, (_) async {});
    const scaler = TextScaler.linear(1.7);
    late MediaQueryData metrics;
    await tester.pumpWidget(
      ChangeNotifierProvider.value(
        value: scale,
        child: MediaQuery(
          data: const MediaQueryData(
            devicePixelRatio: 2,
            textScaler: scaler,
            padding: EdgeInsets.all(20),
            viewPadding: EdgeInsets.all(30),
            viewInsets: EdgeInsets.only(bottom: 80),
            systemGestureInsets: EdgeInsets.all(12),
            displayFeatures: [
              ui.DisplayFeature(
                bounds: Rect.fromLTWH(100, 20, 10, 400),
                type: ui.DisplayFeatureType.hinge,
                state: ui.DisplayFeatureState.postureFlat,
              ),
            ],
            displayCornerRadii: BorderRadius.all(Radius.circular(16)),
          ),
          child: AppViewport(
            child: Builder(
              builder: (context) {
                metrics = MediaQuery.of(context);
                return const SizedBox.expand();
              },
            ),
          ),
        ),
      ),
    );
    expect(metrics.size, screen.screenSize);
    expect(metrics.padding, const EdgeInsets.all(10));
    expect(metrics.viewPadding, const EdgeInsets.all(15));
    expect(metrics.viewInsets.bottom, 40);
    expect(metrics.systemGestureInsets, const EdgeInsets.all(6));
    expect(
      metrics.displayFeatures.single.bounds,
      const Rect.fromLTWH(50, 10, 5, 200),
    );
    expect(
      metrics.displayCornerRadii,
      const BorderRadius.all(Radius.circular(8)),
    );
    expect(metrics.textScaler, same(scaler));
    tester.view.devicePixelRatio = 4;
    await tester.pump();
    expect(metrics.size, screen.screenSize);
    tester.view.physicalSize = Size.zero;
    await tester.pump();
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
    screen.dispose();
    scale.dispose();
  });
}
