import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:rune/widgets/visualizer/lab/lab_section_flow.dart';

void main() {
  Widget flow(List<double> heights, {double? maxHeight}) => Directionality(
    textDirection: TextDirection.ltr,
    child: Align(
      alignment: Alignment.topLeft,
      child: ConstrainedBox(
        constraints: BoxConstraints(maxHeight: maxHeight ?? double.infinity),
        child: LabSectionFlow(
          columnWidth: 100,
          columnGap: 16,
          sectionGap: 10,
          children: [
            for (var i = 0; i < heights.length; i++)
              SizedBox(key: Key('s$i'), height: heights[i]),
          ],
        ),
      ),
    ),
  );

  testWidgets('fitting its content, a column is as wide as its widest '
      'section, and a fill takes that width', (tester) async {
    await tester.pumpWidget(
      Directionality(
        textDirection: TextDirection.ltr,
        child: Align(
          alignment: Alignment.topLeft,
          child: LabSectionFlow(
            columnWidth: 300,
            fitContent: true,
            children: const [
              SizedBox(width: 60, height: 10),
              SizedBox(width: 120, height: 10),
              LabFillWidth(
                child: SizedBox(key: Key('fill'), width: 1000, height: 10),
              ),
            ],
          ),
        ),
      ),
    );
    expect(tester.getSize(find.byType(LabSectionFlow)).width, 120);
    expect(tester.getSize(find.byKey(const Key('fill'))).width, 120);
  });

  Offset at(WidgetTester tester, int i) =>
      tester.getTopLeft(find.byKey(Key('s$i')));

  testWidgets('sections stack in order and a full column starts the next', (
    tester,
  ) async {
    await tester.pumpWidget(flow([40, 40, 40, 60], maxHeight: 100));
    expect(at(tester, 0), const Offset(0, 0));
    expect(at(tester, 1), const Offset(0, 50));
    expect(at(tester, 2), const Offset(116, 0));
    expect(at(tester, 3), const Offset(232, 0));
    // Three columns and two gaps; the tallest column is the first.
    expect(
      tester.getSize(find.byType(LabSectionFlow)),
      const Size(3 * 100 + 2 * 16, 90),
    );
    expect(tester.getSize(find.byKey(const Key('s0'))).width, 100);
  });

  testWidgets('what fits under the height stays in one column', (tester) async {
    await tester.pumpWidget(flow([40, 40], maxHeight: 90));
    expect(at(tester, 1), const Offset(0, 50));
    expect(tester.getSize(find.byType(LabSectionFlow)), const Size(100, 90));
  });

  testWidgets('without a height limit it is one column', (tester) async {
    await tester.pumpWidget(flow([40, 40, 40, 60]));
    expect(at(tester, 3), const Offset(0, 150));
    expect(
      tester.getSize(find.byType(LabSectionFlow)),
      const Size(100, 40 * 3 + 60 + 3 * 10),
    );
  });

  testWidgets('a section taller than the height gets a column and scrolls', (
    tester,
  ) async {
    await tester.pumpWidget(flow([30, 150, 30], maxHeight: 100));
    expect(at(tester, 0), const Offset(0, 0));
    expect(at(tester, 1), const Offset(116, 0));
    expect(at(tester, 2), const Offset(232, 0));
    final flowSize = tester.getSize(find.byType(LabSectionFlow));
    expect(flowSize, const Size(3 * 100 + 2 * 16, 100));
    // The tall section's viewport is as tall as the limit; its content is not.
    expect(
      tester.getSize(find.byType(SingleChildScrollView).at(1)).height,
      100,
    );
    expect(tester.getSize(find.byKey(const Key('s1'))).height, 150);
    await tester.drag(find.byKey(const Key('s1')), const Offset(0, -40));
    await tester.pump();
    expect(at(tester, 1).dy, -40);
  });

  testWidgets('a taller section after others starts its own column', (
    tester,
  ) async {
    await tester.pumpWidget(flow([30, 150], maxHeight: 100));
    expect(at(tester, 1), const Offset(116, 0));
  });

  testWidgets('columns too wide for the room share it, down to the '
      'narrowest they get, and past that are one column that scrolls', (
    tester,
  ) async {
    Widget narrow(double width) => Directionality(
      textDirection: TextDirection.ltr,
      child: Align(
        alignment: Alignment.topLeft,
        child: SizedBox(
          width: width,
          height: 50,
          child: SingleChildScrollView(
            child: LabSectionFlow(
              columnWidth: 100,
              minColumnWidth: 60,
              flowHeight: 50,
              columnGap: 16,
              children: [
                for (var i = 0; i < 3; i++)
                  SizedBox(key: Key('s$i'), height: 40),
              ],
            ),
          ),
        ),
      ),
    );
    // Three columns of 100 need 332; in 250 each is a third of what the
    // gaps leave.
    await tester.pumpWidget(narrow(250));
    expect(tester.getSize(find.byKey(const Key('s0'))).width, 72);
    expect(at(tester, 1), const Offset(88, 0));
    expect(at(tester, 2), const Offset(176, 0));
    expect(tester.getSize(find.byType(LabSectionFlow)).height, 40);
    // Three of 60, the least, need 212: in 150 they are one column, to
    // scroll.
    await tester.pumpWidget(narrow(150));
    expect(tester.getSize(find.byKey(const Key('s0'))).width, 100);
    expect(at(tester, 1), const Offset(0, 50));
    expect(at(tester, 2), const Offset(0, 100));
    await tester.drag(find.byKey(const Key('s0')), const Offset(0, -60));
    await tester.pump();
    expect(at(tester, 2).dy, 40);
    // A column alone is never wider than the room.
    await tester.pumpWidget(
      Directionality(
        textDirection: TextDirection.ltr,
        child: Align(
          alignment: Alignment.topLeft,
          child: SizedBox(
            width: 80,
            child: LabSectionFlow(
              columnWidth: 100,
              children: const [SizedBox(key: Key('s0'), height: 40)],
            ),
          ),
        ),
      ),
    );
    expect(tester.getSize(find.byKey(const Key('s0'))).width, 80);
  });

  testWidgets('no sections is an empty box', (tester) async {
    await tester.pumpWidget(flow([], maxHeight: 100));
    expect(tester.getSize(find.byType(LabSectionFlow)), Size.zero);
  });
}
