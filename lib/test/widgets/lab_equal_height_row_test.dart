import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:rune/widgets/visualizer/lab/lab_equal_height_row.dart';

void main() {
  Widget row(double maxHeight, {double? height}) => Directionality(
    textDirection: TextDirection.ltr,
    child: Align(
      alignment: Alignment.topLeft,
      child: LabEqualHeightRow(
        gap: 10,
        maxHeight: maxHeight,
        height: height,
        children: const [
          SizedBox(key: Key('a'), width: 100, height: 40),
          ColoredBox(
            key: Key('b'),
            color: Color(0xFF000000),
            child: SizedBox(width: 60, height: 120),
          ),
          SizedBox(key: Key('c'), width: 80),
        ],
      ),
    ),
  );

  testWidgets('the children are as tall as the tallest, side by side', (
    tester,
  ) async {
    await tester.pumpWidget(row(500));
    for (final key in ['a', 'b', 'c']) {
      expect(tester.getSize(find.byKey(Key(key))).height, 120, reason: key);
    }
    expect(tester.getTopLeft(find.byKey(const Key('b'))).dx, 110);
    expect(tester.getTopLeft(find.byKey(const Key('c'))).dx, 180);
    expect(
      tester.getSize(find.byType(LabEqualHeightRow)),
      const Size(260, 120),
    );
  });

  testWidgets('no child is taller than the most', (tester) async {
    await tester.pumpWidget(row(90));
    for (final key in ['a', 'b', 'c']) {
      expect(tester.getSize(find.byKey(Key(key))).height, 90, reason: key);
    }
  });

  testWidgets('a height gives every child exactly that, past the most', (
    tester,
  ) async {
    await tester.pumpWidget(row(90, height: 200));
    for (final key in ['a', 'b', 'c']) {
      expect(tester.getSize(find.byKey(Key(key))).height, 200, reason: key);
    }
    expect(
      tester.getSize(find.byType(LabEqualHeightRow)),
      const Size(260, 200),
    );
    await tester.pumpWidget(row(500, height: 30));
    for (final key in ['a', 'b', 'c']) {
      expect(tester.getSize(find.byKey(Key(key))).height, 30, reason: key);
    }
  });

  testWidgets('a height is held to the room the row is given', (tester) async {
    await tester.pumpWidget(
      Directionality(
        textDirection: TextDirection.ltr,
        child: Align(
          alignment: Alignment.topLeft,
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxHeight: 80),
            child: const LabEqualHeightRow(
              gap: 4,
              maxHeight: 500,
              height: 200,
              children: [SizedBox(key: Key('a'), width: 10)],
            ),
          ),
        ),
      ),
    );
    expect(tester.getSize(find.byKey(const Key('a'))).height, 80);
  });

  testWidgets('filling its width, the last child takes what is left, and '
      'no child goes past it', (tester) async {
    Widget filled(double width) => Directionality(
      textDirection: TextDirection.ltr,
      child: Align(
        alignment: Alignment.topLeft,
        child: SizedBox(
          width: width,
          child: LabEqualHeightRow(
            gap: 10,
            maxHeight: 500,
            fill: true,
            children: const [
              SizedBox(key: Key('a'), width: 100, height: 40),
              SizedBox(key: Key('b'), width: 60, height: 20),
            ],
          ),
        ),
      ),
    );
    await tester.pumpWidget(filled(400));
    // 400 less the first child and the gap.
    expect(tester.getSize(find.byKey(const Key('b'))), const Size(290, 40));
    expect(tester.getSize(find.byType(LabEqualHeightRow)).width, 400);
    // With less room than it wants, it takes what is left all the same.
    await tester.pumpWidget(filled(120));
    expect(tester.getSize(find.byKey(const Key('b'))).width, 10);
    // A child before it is no wider than the row.
    await tester.pumpWidget(filled(80));
    expect(tester.getSize(find.byKey(const Key('a'))).width, 80);
    expect(tester.getSize(find.byKey(const Key('b'))).width, 0);
    expect(tester.getSize(find.byType(LabEqualHeightRow)).width, 80);
  });
}
