import 'package:fluent_ui/fluent_ui.dart';
import 'package:fluent_ui/src/controls/form/pickers/pickers.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter/material.dart' as material;

void main() {
  testWidgets('metric changes dismiss only the open combo route', (
    tester,
  ) async {
    final nav = GlobalKey<NavigatorState>();
    Widget app(double ratio) => FluentApp(
      navigatorKey: nav,
      builder: (context, child) => MediaQuery(
        data: MediaQuery.of(context).copyWith(devicePixelRatio: ratio),
        child: child!,
      ),
      home: Center(
        child: ComboBox<int>(
          value: 1,
          items: const [
            ComboBoxItem(value: 1, child: Text('one')),
            ComboBoxItem(value: 2, child: Text('two')),
          ],
          onChanged: (_) {},
        ),
      ),
    );
    await tester.pumpWidget(app(1));
    await tester.tap(find.byType(ComboBox<int>));
    await tester.pumpAndSettle();
    expect(find.text('two'), findsOneWidget);
    nav.currentState!.push(
      PageRouteBuilder<void>(
        opaque: false,
        pageBuilder: (_, __, ___) => const Text('other route'),
      ),
    );
    await tester.pumpAndSettle();
    await tester.pumpWidget(app(2));
    await tester.pumpAndSettle();
    expect(find.text('two'), findsNothing);
    expect(find.text('other route'), findsOneWidget);
  });

  testWidgets('metric changes dismiss picker and suggestions', (tester) async {
    final suggestion = GlobalKey<AutoSuggestBoxState<int>>();
    Widget app(double ratio) => FluentApp(
      builder: (context, child) => MediaQuery(
        data: MediaQuery.of(context).copyWith(devicePixelRatio: ratio),
        child: child!,
      ),
      home: Column(
        children: [
          Picker(
            pickerHeight: 100,
            pickerContent: (_) => const Text('picker content'),
            child: (_, open) =>
                Button(onPressed: open, child: const Text('picker')),
          ),
          AutoSuggestBox<int>(
            key: suggestion,
            items: [AutoSuggestBoxItem(value: 1, label: 'suggestion')],
          ),
        ],
      ),
    );
    await tester.pumpWidget(app(1));
    await tester.tap(find.text('picker'));
    await tester.pumpAndSettle();
    expect(find.text('picker content'), findsOneWidget);
    await tester.pumpWidget(app(2));
    await tester.pumpAndSettle();
    expect(find.text('picker content'), findsNothing);
    suggestion.currentState!.showOverlay();
    await tester.pump();
    expect(suggestion.currentState!.isOverlayVisible, isTrue);
    await tester.pumpWidget(app(3));
    await tester.pump();
    await tester.pump();
    expect(suggestion.currentState!.isOverlayVisible, isFalse);
  });
  testWidgets('slider label edge shift stays in overlay coordinates', (
    tester,
  ) async {
    Future<Rect> paintAt(double factor, Offset translation) async {
      await tester.pumpWidget(
        Transform.translate(
          offset: translation,
          child: Transform.scale(
            scale: factor,
            alignment: Alignment.topLeft,
            child: FluentApp(
              home: Align(
                alignment: Alignment.topLeft,
                child: SizedBox(
                  width: 150,
                  child: Slider(
                    value: 0,
                    label: 'long value label',
                    onChanged: (_) {},
                  ),
                ),
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      final slider = tester.element(find.byType(material.Slider));
      final box = slider.findRenderObject()! as RenderBox;
      final shape = material.SliderTheme.of(slider).valueIndicatorShape!;
      final label = TextPainter(
        text: const TextSpan(
          text: 'long value label',
          style: TextStyle(fontSize: 14),
        ),
        textDirection: TextDirection.ltr,
      )..layout();
      Rect? bounds;
      void paint(PaintingContext context, Offset offset) {
        shape.paint(
          context,
          const Offset(10, 30),
          activationAnimation: const AlwaysStoppedAnimation(1),
          enableAnimation: const AlwaysStoppedAnimation(1),
          isDiscrete: false,
          labelPainter: label,
          parentBox: box,
          sliderTheme: material.SliderTheme.of(slider),
          textDirection: TextDirection.ltr,
          value: 0,
          textScaleFactor: 1,
          sizeWithOverflow: const Size(800, 600),
        );
      }

      expect(
        paint,
        paints..something((name, args) {
          if (name != #drawPath) return false;
          bounds = (args[0] as Path).getBounds();
          return true;
        }),
      );
      return bounds!;
    }

    final normal = await paintAt(1, Offset.zero);
    final transformed = await paintAt(1.5, const Offset(70, 25));
    expect(transformed.left, closeTo(normal.left, 0.001));
    expect(transformed.width, closeTo(normal.width, 0.001));
  });
}
