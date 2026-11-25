import 'package:flutter_test/flutter_test.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/widgets/visualizer/lab/lab_data.dart';
import 'package:rune/widgets/visualizer/lab/lab_plan_data.dart';
import 'package:rune/widgets/visualizer/lab/lab_rows_painter.dart';

void main() {
  test('plan enum choices draw the matching section, cut, and moment', () {
    const plan = VisualizerPlan(
      length: 20,
      sections: [
        VisualizerPlanSection(
          start: 0,
          end: 20,
          label: 'A',
          tier: VisualizerSectionTier.high,
        ),
      ],
      cuts: [
        VisualizerPlanCut(
          time: 4,
          shot: 'named-by-scene',
          size: VisualizerShotSize.top,
          fill: 0,
          strong: false,
          dress: 'scene-transition',
          first: false,
        ),
      ],
      looks: [],
      moments: [
        VisualizerPlanMoment(
          time: 8,
          strength: VisualizerMomentStrength.strong,
          effect: 'scene-effect',
        ),
      ],
    );

    final section = labSectionsOf(plan).single;
    final marks = labPlanMarksOf(plan);
    expect(section.tier, LabTier.high);
    expect(labTierColor(section.tier), isNot(labTierColor(LabTier.quiet)));
    expect(marks.first.size, VisualizerShotSize.top);
    expect(marks.first.shotName, 'named-by-scene');
    expect(marks.first.dress, 'scene-transition');
    expect(marks.last.strong, isTrue);
    expect(marks.last.effect, 'scene-effect');
  });
}
