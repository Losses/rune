import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:rune/widgets/visualizer/lab/lab_data.dart';
import 'package:rune/widgets/visualizer/lab/lab_plan_data.dart';
import 'package:rune/widgets/visualizer/lab/lab_rows.dart';
import 'package:rune/widgets/visualizer/lab/lab_series.dart';

const _nan = double.nan;

LabSeriesCatalogue _catalogue({int version = 1}) => LabSeriesCatalogue(
  version: version,
  rate: 20,
  columns: const [
    LabSeriesColumn(
      id: 'shot',
      name: 'Shot',
      group: 'Director',
      kind: 'category',
      categories: ['wide', 'close'],
    ),
    LabSeriesColumn(
      id: 'drive',
      name: 'Drive',
      group: 'Signals',
      kind: 'number',
    ),
    LabSeriesColumn(
      id: 'band_low',
      name: 'band_low',
      group: 'Signals',
      kind: 'number',
      hi: 2,
      plan: 'no',
    ),
    LabSeriesColumn(
      id: 'age',
      name: 'Age',
      group: 'Director',
      kind: 'number',
      hi: 8,
      unit: 's',
      plan: 'later',
    ),
  ],
);

LabSeriesSample _sample(int bucket, List<double> values, {int version = 1}) =>
    LabSeriesSample(bucket: bucket, columnsVersion: version, values: values);

void main() {
  group('series store', () {
    test('the catalogue maps kinds and plan words', () {
      final store = LabSeriesStore()..setCatalogue(_catalogue());
      final ids = [for (final c in store.columns) c.id];
      expect(ids, ['sections', 'marks', 'shot', 'drive', 'band_low', 'age']);
      LabColumn col(String id) => store.columns.firstWhere((c) => c.id == id);
      expect(col('shot').kind, LabColumnKind.category);
      expect(col('shot').categories, ['wide', 'close']);
      expect(col('drive').kind, LabColumnKind.number);
      expect(col('drive').unit, isNull);
      expect(col('drive').binds, isEmpty);
      expect(col('band_low').plan, LabPlanAvailability.no);
      expect(col('band_low').hi, 2);
      // An unknown word is read as no plan.
      expect(col('age').plan, LabPlanAvailability.no);
      expect(col('age').unit, 's');
      expect(labColumnKindOf('event'), LabColumnKind.event);
      expect(labColumnKindOf('bool'), LabColumnKind.bool);
      expect(labColumnKindOf('phase'), LabColumnKind.phase);
      expect(labColumnKindOf('something else'), LabColumnKind.number);
    });

    test('a newer sample overwrites the bucket', () {
      final store = LabSeriesStore()
        ..setCatalogue(_catalogue())
        ..setBuckets(10);
      store.addSamples([
        _sample(2, [1, 0.5, 0.1, 3]),
        _sample(3, [0, 0.6, 0.2, 4]),
      ]);
      var drive = store.actualSeries['drive']!;
      expect(drive.length, 10);
      expect(drive[2], closeTo(0.5, 1e-6));
      expect(drive[0].isNaN, isTrue);
      store.addSamples([
        _sample(2, [1, 0.9, 0.1, 3]),
      ]);
      drive = store.actualSeries['drive']!;
      expect(drive[2], closeTo(0.9, 1e-6));
      expect(drive[3], closeTo(0.6, 1e-6));
    });

    test('a sample of another catalogue or past the song is left out', () {
      final store = LabSeriesStore()
        ..setCatalogue(_catalogue())
        ..setBuckets(5);
      store.addSamples([
        _sample(1, [1, 1, 1, 1], version: 2),
        _sample(7, [1, 1, 1, 1]),
        _sample(-1, [1, 1, 1, 1]),
      ]);
      expect(store.actualSeries['drive']!.every((v) => v.isNaN), isTrue);
    });

    test('without a known length the series grow with the samples', () {
      final store = LabSeriesStore()..setCatalogue(_catalogue());
      store.addSamples([
        _sample(100, [0, 0.4, 0, 0]),
      ]);
      final drive = store.actualSeries['drive']!;
      expect(drive.length, greaterThan(100));
      expect(drive[100], closeTo(0.4, 1e-6));
      expect(drive[99].isNaN, isTrue);
    });

    test('the series are live buffers, and the revision tells a change', () {
      final store = LabSeriesStore()
        ..setCatalogue(_catalogue())
        ..setBuckets(4);
      final first = store.actualSeries;
      final drive = first['drive']!;
      final revision = store.revision;
      // No change, no new revision, and the same map.
      expect(identical(store.actualSeries, first), isTrue);
      expect(store.revision, revision);
      store.addSamples([
        _sample(1, [0, 0.2, 0, 0]),
      ]);
      expect(store.revision, isNot(revision));
      // The buffer itself took the value: nothing was copied.
      expect(identical(store.actualSeries['drive'], drive), isTrue);
      expect(drive[1], closeTo(0.2, 1e-6));
      // A sample that is left out changes nothing.
      final later = store.revision;
      store.addSamples([
        _sample(9, [1, 1, 1, 1]),
      ]);
      expect(store.revision, later);
      // New buffers come with a new map and a new revision.
      store.setBuckets(8);
      expect(identical(store.actualSeries, first), isFalse);
      expect(store.actualSeries['drive']!.length, 8);
      expect(store.actualSeries['drive']![1], closeTo(0.2, 1e-6));
      expect(store.revision, isNot(later));
    });

    test('the series are as long as the longer of the plan and the player', () {
      LabSeriesPlan plan(int buckets) => LabSeriesPlan(
        columnsVersion: 1,
        rate: 20,
        buckets: buckets,
        values: List.filled(4 * buckets, 0),
      );
      final store = LabSeriesStore()..setCatalogue(_catalogue());
      // The player runs on past the end of the chart.
      store.setBuckets(10);
      expect(store.importPlan(plan(6)), isTrue);
      expect(store.actualSeries['drive']!.length, 10);
      expect(store.planSeries['drive']!.length, 6);
      store.addSamples([
        _sample(8, [0, 0.7, 0, 0]),
        _sample(10, [0, 0.9, 0, 0]),
      ]);
      expect(store.actualSeries['drive']![8], closeTo(0.7, 1e-6));
      // Past the limit is left out.
      expect(store.actualSeries['drive']!.length, 10);

      // A plan longer than the player's duration wins.
      expect(store.importPlan(plan(14)), isTrue);
      expect(store.actualSeries['drive']!.length, 14);
      expect(store.actualSeries['drive']![8], closeTo(0.7, 1e-6));

      // A catalogue that comes later keeps both.
      final other = LabSeriesStore()
        ..setBuckets(10)
        ..setCatalogue(_catalogue());
      expect(other.importPlan(plan(6)), isTrue);
      expect(other.actualSeries['drive']!.length, 10);
    });

    test('another catalogue version starts afresh', () {
      final store = LabSeriesStore()
        ..setCatalogue(_catalogue())
        ..setBuckets(4);
      store.addSamples([
        _sample(0, [0, 0.1, 0, 0]),
      ]);
      // The same version changes nothing.
      store.setCatalogue(_catalogue());
      expect(store.actualSeries['drive']![0], closeTo(0.1, 1e-6));
      store.setCatalogue(_catalogue(version: 2));
      expect(store.version, 2);
      expect(store.actualSeries['drive']!.every((v) => v.isNaN), isTrue);
      expect(store.planSeries, isEmpty);
    });

    test('a reset forgets the song', () {
      final store = LabSeriesStore()
        ..setCatalogue(_catalogue())
        ..setBuckets(4);
      store.reset();
      expect(store.version, 0);
      expect(store.columns.map((c) => c.id), ['sections', 'marks']);
      expect(store.actualSeries, isEmpty);
      // Samples without a catalogue are left out.
      store.addSamples([
        _sample(0, [1, 1, 1, 1]),
      ]);
      expect(store.actualSeries, isEmpty);
    });

    test('the about and binds of a column reach the row', () {
      final store = LabSeriesStore()
        ..setCatalogue(
          const LabSeriesCatalogue(
            version: 1,
            rate: 20,
            columns: [
              LabSeriesColumn(
                id: 'input:glow',
                name: 'Glow',
                group: 'Inputs',
                kind: 'number',
                binds: ['drive', 'input:pulse'],
              ),
            ],
          ),
        );
      final glow = store.columns.firstWhere((c) => c.id == 'input:glow');
      expect(glow.binds, ['drive', 'input:pulse']);
    });

    test('the plan is imported column-major, for the columns that have it', () {
      final store = LabSeriesStore()..setCatalogue(_catalogue());
      // Four columns, three buckets.
      final ok = store.importPlan(
        const LabSeriesPlan(
          columnsVersion: 1,
          rate: 20,
          buckets: 3,
          values: [
            0, 1, 0, // shot
            0.1, 0.2, 0.3, // drive
            9, 9, 9, // band_low (no plan)
            _nan, _nan, _nan, // age (no plan)
          ],
        ),
      );
      expect(ok, isTrue);
      expect(store.planSeries.keys, ['shot', 'drive']);
      expect(store.planSeries['shot'], Float32List.fromList([0, 1, 0]));
      expect(store.planSeries['drive']![2], closeTo(0.3, 1e-6));
      // The actual series are as long as the plan.
      expect(store.actualSeries['drive']!.length, 3);
    });

    test('a plan of another catalogue or too short is left out', () {
      final store = LabSeriesStore()..setCatalogue(_catalogue());
      expect(
        store.importPlan(
          const LabSeriesPlan(
            columnsVersion: 5,
            rate: 20,
            buckets: 1,
            values: [0, 0, 0, 0],
          ),
        ),
        isFalse,
      );
      expect(
        store.importPlan(
          const LabSeriesPlan(
            columnsVersion: 1,
            rate: 20,
            buckets: 3,
            values: [0, 0, 0],
          ),
        ),
        isFalse,
      );
      expect(store.planSeries, isEmpty);
    });
  });

  group('default rows', () {
    test('the contract ids are on', () {
      const d = LabRowSelection.defaults;
      for (final id in [
        'marks',
        'shot',
        'look',
        'level',
        'drive',
        'intensity',
        'hit',
        'hits',
        'low',
        'mid',
        'high',
      ]) {
        expect(d.planOn(id) && d.actualOn(id), isTrue, reason: id);
      }
      expect(d.planOn('sections'), isTrue);
      expect(d.planOn('band_low'), isFalse);
      expect(d.actualOn('band_low'), isTrue);
      expect(d.planOn('bass'), isFalse);
      expect(d.actualOn('lheld'), isFalse);
    });

    test('every input is on unless it was told otherwise', () {
      const d = LabRowSelection.defaults;
      expect(d.planOn('input:glow') && d.actualOn('input:glow'), isTrue);
      final off = d.setPart('input:glow', plan: false);
      expect(off.planOn('input:glow'), isFalse);
      expect(off.actualOn('input:glow'), isTrue);
      final back = LabRowSelection.fromJson(off.toJson());
      expect(back.planOn('input:glow'), isFalse);
      expect(back.actualOn('input:other'), isTrue);
    });
  });

  test('marks merge in time order', () {
    LabMark manual(double t) => LabMark.manual(time: t, label: 'x');
    final merged = labMergeMarks(
      [manual(1), manual(4)],
      [manual(2), manual(3), manual(5)],
    );
    expect([for (final m in merged) m.time], [1, 2, 3, 4, 5]);
  });
}
