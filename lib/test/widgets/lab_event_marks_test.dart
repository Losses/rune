import 'package:flutter_test/flutter_test.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/providers/visualizer_lab.dart';
import 'package:rune/widgets/visualizer/lab/lab_data.dart';
import 'package:rune/widgets/visualizer/lab/lab_series_adapter.dart';
import 'package:rune/widgets/visualizer/lab/lab_tooltip.dart';

VisualizerLabEvent _cut(double time, {bool first = false}) =>
    VisualizerLabEvent(
      time: time,
      kind: VisualizerLabEventKindCut(
        shot: 'alley',
        size: 'medium',
        fill: 0,
        strong: false,
        dress: null,
        first: first,
      ),
    );

void main() {
  group('marks of events', () {
    test('a first cut where the song time jumped is a seek', () {
      final back = labMarkOfEvent(_cut(10, first: true), 83.4)!;
      expect(back.kind, LabMarkKind.jump);
      expect(back.time, 10);
      expect(back.from, 83.4);

      final forward = labMarkOfEvent(_cut(120, first: true), 83.4)!;
      expect(forward.kind, LabMarkKind.jump);
      expect(forward.from, 83.4);

      // A move forward of more than a second is a seek, as the engine counts.
      final near = labMarkOfEvent(_cut(84.6, first: true), 83.4)!;
      expect(near.kind, LabMarkKind.jump);
    });

    test('a first cut after a scene switch is an ordinary cut', () {
      // The song went on from where it was, within a poll or two.
      for (final time in [83.4, 83.5, 84.2, 84.4, 83.2]) {
        final mark = labMarkOfEvent(_cut(time, first: true), 83.4)!;
        expect(mark.kind, LabMarkKind.cut, reason: '$time');
        expect(mark.shotName, 'alley');
        expect(mark.size, 'medium');
        expect(mark.first, isTrue);
      }
      final tip = labDescribeMark(
        labMarkOfEvent(_cut(83.5, first: true), 83.4)!,
      );
      expect(tip, startsWith('Cut '));
      expect(tip, isNot(contains('Seek')));
      expect(tip, contains('first'));
    });

    test('a first cut with no song time before it is an ordinary cut', () {
      expect(labMarkOfEvent(_cut(5, first: true), null)!.kind, LabMarkKind.cut);
    });

    test('a mark of an unnamed shot or look still reads well', () {
      final cut = LabMark.cut(time: 12.3, shotName: '', size: 'wide');
      expect(labDescribeMark(cut), 'Cut 0:12.3  unnamed shot (wide)');
      final look = LabMark.look(time: 5, lookName: '');
      expect(labDescribeMark(look), 'Look 0:05.0  unnamed look');
    });

    test('a cut that is not the first is never a seek', () {
      expect(labMarkOfEvent(_cut(10), 83.4)!.kind, LabMarkKind.cut);
    });

    test('a cut played with a transition is marked as a cut wearing it', () {
      expect(labPlayedLabel(const VisualizerLabCommandCut()), 'Cut');
      expect(
        labPlayedLabel(
          const VisualizerLabCommandCutDressed(
            transition: VisualizerLabTransition.fade,
          ),
        ),
        'Fade Cut',
      );
      expect(labPlayedLabel(const VisualizerLabCommandLensGlitch()), 'Glitch');
    });

    test('a moment tells whether its effect dresses a cut', () {
      const onCut = LabMark.moment(time: 60, strong: true, effect: 'glitch');
      const over = LabMark.moment(time: 70, strong: true, effect: 'glitch');
      const marks = [
        onCut,
        LabMark.cut(
          time: 60,
          shotName: 'overhead',
          size: 'top',
          strong: true,
          dress: 'glitch',
        ),
        over,
        // A cut with no transition at the same time does not wear the effect.
        LabMark.cut(time: 70, shotName: 'alley', size: 'medium'),
      ];
      expect(labMomentOnCut(marks, onCut), isTrue);
      expect(labMomentOnCut(marks, over), isFalse);
      expect(
        labDescribeMark(onCut, onCut: true),
        'Moment 1:00.0  strong  glitch on the cut',
      );
      expect(
        labDescribeMark(over),
        'Moment 1:10.0  strong  glitch over the picture',
      );
    });
  });
}
