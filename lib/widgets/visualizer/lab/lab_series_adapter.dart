import '../../../bindings/bindings.dart';

import 'lab_data.dart';
import 'lab_series.dart';

/// What a [VisualizerLabResponse] adds to the lab's series, as input of the
/// [LabSeriesStore]: the response's column catalogue, when it has one, and its
/// finished buckets, oldest first, each with the catalogue version it follows.
LabSeriesUpdate labSeriesUpdateFromHub(VisualizerLabResponse response) {
  final columns = response.columns;
  return LabSeriesUpdate(
    catalogue: columns == null
        ? null
        : LabSeriesCatalogue(
            version: columns.version.toInt(),
            rate: columns.rate,
            columns: [
              for (final c in columns.columns)
                LabSeriesColumn(
                  id: c.id,
                  name: c.name,
                  group: c.group,
                  kind: c.kind,
                  lo: c.lo,
                  hi: c.hi,
                  unit: c.unit,
                  categories: c.categories,
                  plan: c.plan,
                  binds: c.binds,
                  rest: c.rest,
                ),
            ],
          ),
    samples: [
      for (final s in response.samples)
        LabSeriesSample(
          bucket: s.bucket,
          columnsVersion: s.columnsVersion.toInt(),
          values: s.values,
        ),
    ],
  );
}

/// The plan's series in a [VisualizerPlan], as a [LabSeriesPlan]: the
/// catalogue version, the rate, the number of buckets and the column-major
/// values. It gives null for a plan that has no series.
LabSeriesPlan? labSeriesPlanFromHub(VisualizerPlan plan) {
  final series = plan.series;
  if (series == null) return null;
  return LabSeriesPlan(
    columnsVersion: series.columnsVersion.toInt(),
    rate: series.rate,
    buckets: series.buckets,
    values: series.values,
  );
}

/// A song time this much behind the last one is a seek back.
const labSeekBackwards = 0.3;

/// A song time more than this ahead of the last one is a seek forward, as the
/// visualizer's own engine counts it. The poll gap, even with the overlay
/// closed, is below it.
const labSeekForwards = 1.0;

/// The mark of an event, given the song time [last] the lab knew before it,
/// or null for an event with no mark.
///
/// The first cut of a scene is a seek only where the song time really jumped
/// from [last]: the engine also sets it on the first shot after an automatic
/// scene switch, which is an ordinary cut.
LabMark? labMarkOfEvent(VisualizerLabEvent event, double? last) {
  final kind = event.kind;
  if (kind is VisualizerLabEventKindCut) {
    if (kind.first &&
        last != null &&
        (event.time < last - labSeekBackwards ||
            event.time > last + labSeekForwards)) {
      return LabMark.jump(time: event.time, from: last);
    }
    return LabMark.cut(
      time: event.time,
      shotName: kind.shot,
      size: kind.size,
      fill: kind.fill,
      strong: kind.strong,
      dress: kind.dress,
      first: kind.first,
    );
  } else if (kind is VisualizerLabEventKindLook) {
    return LabMark.look(time: event.time, lookName: kind.look);
  } else if (kind is VisualizerLabEventKindMoment) {
    return LabMark.moment(
      time: event.time,
      strong: kind.strength == 'strong',
      effect: kind.effect,
    );
  }
  return null;
}
