import 'dart:ui';

import 'package:flutter_test/flutter_test.dart';

import 'package:rune/widgets/visualizer/lab/lab_layout_mode.dart';

void main() {
  test('a desktop or a tablet window that is tall enough has the cards '
      'above the timeline', () {
    expect(labLayoutFor(const Size(1280, 800)), LabLayout.together);
    expect(labLayoutFor(const Size(768, 1024)), LabLayout.together);
    expect(labLayoutFor(const Size(1024, 768)), LabLayout.together);
    expect(
      labLayoutFor(const Size(651, labTogetherMinHeight)),
      LabLayout.together,
    );
  });

  test('a phone upright has pages down the window', () {
    expect(labLayoutFor(const Size(390, 844)), LabLayout.pagesDown);
    expect(labLayoutFor(const Size(650, 900)), LabLayout.pagesDown);
  });

  test(
    'a phone on its side, wide as a desktop and short, has pages across',
    () {
      expect(labLayoutFor(const Size(844, 390)), LabLayout.pagesAcross);
      expect(
        labLayoutFor(const Size(1280, labTogetherMinHeight - 1)),
        LabLayout.pagesAcross,
      );
    },
  );

  test('a narrow window wider than tall has pages across', () {
    expect(labLayoutFor(const Size(640, 600)), LabLayout.pagesAcross);
  });
}
