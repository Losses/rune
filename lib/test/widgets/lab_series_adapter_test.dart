import 'package:flutter_test/flutter_test.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/widgets/visualizer/lab/lab_data.dart';
import 'package:rune/widgets/visualizer/lab/lab_series.dart';
import 'package:rune/widgets/visualizer/lab/lab_series_adapter.dart';

void main() {
  test('the hub column enums reach the lab catalogue and its filters', () {
    final response = VisualizerLabResponse(
      inspecting: true,
      songTime: 0,
      planVersion: Uint64.fromBigInt(BigInt.one),
      events: const [],
      columns: VisualizerLabColumns(
        version: Uint64.fromBigInt(BigInt.one),
        rate: 20,
        columns: const [
          VisualizerLabColumn(
            id: 'shot',
            name: 'Shot',
            group: VisualizerLabColumnGroup.director,
            kind: VisualizerLabColumnKind.category,
            lo: 0,
            hi: 1,
            unit: '',
            categories: ['wide', 'top'],
            plan: VisualizerLabColumnPlan.yes,
            binds: [],
          ),
          VisualizerLabColumn(
            id: 'audio',
            name: 'Audio',
            group: VisualizerLabColumnGroup.inputs,
            kind: VisualizerLabColumnKind.phase,
            lo: 0,
            hi: 1,
            unit: '',
            categories: [],
            plan: VisualizerLabColumnPlan.no,
            binds: [],
          ),
        ],
      ),
      samples: const [],
    );

    final update = labSeriesUpdateFromHub(response);
    final store = LabSeriesStore()..setCatalogue(update.catalogue!);
    final shot = store.columns.singleWhere((column) => column.id == 'shot');
    final audio = store.columns.singleWhere((column) => column.id == 'audio');
    expect(shot.group, 'Director');
    expect(shot.kind, LabColumnKind.category);
    expect(shot.plan, LabPlanAvailability.yes);
    expect(audio.group, 'Inputs');
    expect(audio.kind, LabColumnKind.phase);
    expect(audio.plan, LabPlanAvailability.no);
  });

  test(
    'the event enums draw the matching mark and preserve scene identifiers',
    () {
      final event = VisualizerLabEvent(
        time: 12,
        kind: VisualizerLabEventKindMoment(
          strength: VisualizerMomentStrength.strong,
          effect: 'scene-effect-name',
        ),
      );
      final mark = labMarkOfEvent(event, null)!;
      expect(mark.kind, LabMarkKind.moment);
      expect(mark.strong, isTrue);
      expect(mark.effect, 'scene-effect-name');
    },
  );
}
