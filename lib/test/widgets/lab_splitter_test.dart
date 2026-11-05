import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:rune/widgets/visualizer/lab/lab_splitter.dart';

final _dark = FluentThemeData(brightness: Brightness.dark);

// The lab is dark, and the splitter takes its colours from the theme.
Widget _host(Widget child) => FluentTheme(
  data: _dark,
  child: Directionality(textDirection: TextDirection.ltr, child: child),
);

void main() {
  final calls = <(double?, bool)>[];

  Future<Offset> pumpSplitter(WidgetTester tester) async {
    calls.clear();
    await tester.pumpWidget(
      _host(
        Align(
          alignment: Alignment.topLeft,
          child: SizedBox(
            width: 300,
            child: LabSplitter(
              currentSize: () => 200,
              minSize: 96,
              maxSize: 400,
              onSize: (size, {required save}) => calls.add((size, save)),
            ),
          ),
        ),
      ),
    );
    return tester.getCenter(find.byType(LabSplitter));
  }

  testWidgets('it is a thin bar with the row resize cursor', (tester) async {
    final center = await pumpSplitter(tester);
    expect(
      tester.getSize(find.byType(LabSplitter)).height,
      labSplitterThickness,
    );
    final region = tester.widget<MouseRegion>(
      find.descendant(
        of: find.byType(LabSplitter),
        matching: find.byType(MouseRegion),
      ),
    );
    expect(region.cursor, SystemMouseCursors.resizeRow);
    expect(center.dy, labSplitterThickness / 2);
  });

  testWidgets('its grip alone answers the pointer: brighter under it, the '
      'accent when pressed', (tester) async {
    final center = await pumpSplitter(tester);
    final res = _dark.resources;
    final accent = _dark.accentColor.defaultBrushFor(Brightness.dark);
    // The grip is the only thing it fills: the space around it stays as it
    // is.
    Color? grip() {
      final boxes = tester
          .widgetList<AnimatedContainer>(
            find.descendant(
              of: find.byType(LabSplitter),
              matching: find.byType(AnimatedContainer),
            ),
          )
          .toList();
      expect(boxes, hasLength(1));
      return (boxes.single.decoration as BoxDecoration?)?.color;
    }

    double length() => tester
        .getSize(
          find.descendant(
            of: find.byType(LabSplitter),
            matching: find.byType(AnimatedContainer),
          ),
        )
        .width;

    expect(grip(), res.controlStrongFillColorDisabled);
    final rest = length();
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    addTearDown(mouse.removePointer);
    await mouse.addPointer(location: center + const Offset(0, 100));
    await mouse.moveTo(center);
    await tester.pumpAndSettle();
    expect(grip(), res.controlStrongFillColorDefault);
    expect(length(), greaterThan(rest));
    await mouse.down(center);
    await tester.pump();
    expect(grip(), accent);
    // Still pressed when the drag leaves it.
    await mouse.moveTo(center + const Offset(0, 100));
    await tester.pump();
    expect(grip(), accent);
    await mouse.up();
    await tester.pumpAndSettle();
    expect(grip(), res.controlStrongFillColorDisabled);
    expect(length(), rest);
  });

  testWidgets('dragging sets the height from where it started, within limits', (
    tester,
  ) async {
    final center = await pumpSplitter(tester);
    final pointer = TestPointer(1);
    await tester.sendEventToBinding(pointer.down(center));
    await tester.sendEventToBinding(pointer.move(center + const Offset(0, 50)));
    expect(calls.last, (250.0, false));
    await tester.sendEventToBinding(
      pointer.move(center + const Offset(0, 900)),
    );
    expect(calls.last, (400.0, false));
    await tester.sendEventToBinding(
      pointer.move(center + const Offset(0, -900)),
    );
    expect(calls.last, (96.0, false));
    await tester.sendEventToBinding(pointer.up());
    expect(calls.last, (96.0, true));
    expect(calls.where((c) => c.$2).length, 1);
  });

  testWidgets('a press that does not move saves nothing', (tester) async {
    final center = await pumpSplitter(tester);
    final pointer = TestPointer(1);
    await tester.sendEventToBinding(pointer.down(center));
    await tester.sendEventToBinding(pointer.up());
    expect(calls, isEmpty);
  });

  testWidgets('a double press goes back to automatic', (tester) async {
    final center = await pumpSplitter(tester);
    final pointer = TestPointer(1);
    await tester.sendEventToBinding(pointer.down(center));
    await tester.sendEventToBinding(pointer.up());
    await tester.sendEventToBinding(
      pointer.down(center, timeStamp: const Duration(milliseconds: 300)),
    );
    expect(calls, [(null, true)]);
    await tester.sendEventToBinding(pointer.up());
  });

  testWidgets('two presses far apart are not a double press', (tester) async {
    final center = await pumpSplitter(tester);
    final pointer = TestPointer(1);
    await tester.sendEventToBinding(pointer.down(center));
    await tester.sendEventToBinding(pointer.up());
    await tester.sendEventToBinding(
      pointer.down(center, timeStamp: const Duration(milliseconds: 400)),
    );
    await tester.sendEventToBinding(pointer.up());
    expect(calls, isEmpty);
  });

  testWidgets('across, it sets a width from a drag to the side', (
    tester,
  ) async {
    calls.clear();
    await tester.pumpWidget(
      _host(
        Align(
          alignment: Alignment.topLeft,
          child: SizedBox(
            height: 200,
            child: LabSplitter(
              axis: Axis.horizontal,
              currentSize: () => 300,
              minSize: 120,
              maxSize: 500,
              onSize: (size, {required save}) => calls.add((size, save)),
            ),
          ),
        ),
      ),
    );
    final splitter = find.byType(LabSplitter);
    expect(tester.getSize(splitter), const Size(labSplitterThickness, 200));
    final region = tester.widget<MouseRegion>(
      find.descendant(of: splitter, matching: find.byType(MouseRegion)),
    );
    expect(region.cursor, SystemMouseCursors.resizeColumn);
    final center = tester.getCenter(splitter);
    final pointer = TestPointer(1);
    await tester.sendEventToBinding(pointer.down(center));
    // Down does nothing across.
    await tester.sendEventToBinding(pointer.move(center + const Offset(0, 40)));
    expect(calls.last, (300.0, false));
    await tester.sendEventToBinding(pointer.move(center + const Offset(60, 0)));
    expect(calls.last, (360.0, false));
    await tester.sendEventToBinding(pointer.up());
    expect(calls.last, (360.0, true));
  });

  testWidgets('across and left to itself, it sets no height of its own', (
    tester,
  ) async {
    await tester.pumpWidget(
      _host(
        Align(
          alignment: Alignment.topLeft,
          child: LabSplitter(
            axis: Axis.horizontal,
            currentSize: () => 0,
            minSize: 0,
            maxSize: 1,
            onSize: (_, {required save}) {},
          ),
        ),
      ),
    );
    expect(tester.getSize(find.byType(LabSplitter)).height, lessThan(50));
  });
}
