import 'package:flutter_test/flutter_test.dart';

import 'package:rune/widgets/visualizer/lab/lab_rows_painter.dart';
import 'package:rune/widgets/visualizer/lab/lab_timeline_math.dart';

LabGeometry _geometry(double width, {double? labelWidth}) => LabGeometry(
  width: width,
  height: 400,
  view: LabView.whole(100),
  scrollY: 0,
  length: 100,
  labelWidth: labelWidth,
);

void main() {
  group('label width', () {
    test('without a width it is 108 under 560 px and 150 from there', () {
      expect(_geometry(400).label, 108);
      expect(_geometry(559).label, 108);
      expect(_geometry(560).label, 150);
      expect(_geometry(1400).label, 150);
    });

    test('a set width is kept inside 88 and the smaller of 420 and half', () {
      expect(_geometry(1400, labelWidth: 200).label, 200);
      expect(_geometry(1400, labelWidth: 10).label, 88);
      expect(_geometry(1400, labelWidth: 900).label, 420);
      expect(_geometry(600, labelWidth: 900).label, 300);
      // A window so narrow that half of it is under the least.
      expect(_geometry(120, labelWidth: 100).label, 88);
    });

    test('the plot takes what the label column leaves', () {
      final g = _geometry(1000, labelWidth: 300);
      expect(g.plotWidth, 1000 - 300 - LabGeometry.padRight);
      expect(g.xOf(g.timeOf(500)), closeTo(500, 1e-9));
      expect(g.xOf(0), 300);
    });
  });
}
