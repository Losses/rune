import '../../../bindings/bindings.dart';

import 'lab_data.dart';

/// The sections of a plan.
List<LabSection> labSectionsOf(VisualizerPlan plan) => List.unmodifiable([
  for (final s in plan.sections)
    LabSection(start: s.start, end: s.end, label: s.label, tier: s.tier),
]);

/// The cuts, looks and moments of a plan as marks, sorted by time.
List<LabMark> labPlanMarksOf(VisualizerPlan plan) {
  final marks = <LabMark>[
    for (final c in plan.cuts)
      LabMark.cut(
        time: c.time,
        shotName: c.shot,
        size: c.size,
        fill: c.fill,
        strong: c.strong,
        dress: c.dress,
        first: c.first,
      ),
    for (final l in plan.looks) LabMark.look(time: l.time, lookName: l.look),
    for (final m in plan.moments)
      LabMark.moment(
        time: m.time,
        strong: m.strength == VisualizerMomentStrength.strong,
        effect: m.effect,
      ),
  ];
  // The sort is stable in effect: equal times keep the order of the kinds.
  final indexed = [for (var i = 0; i < marks.length; i++) (i, marks[i])]
    ..sort((a, b) {
      final t = a.$2.time.compareTo(b.$2.time);
      return t != 0 ? t : a.$1.compareTo(b.$1);
    });
  return List.unmodifiable([for (final e in indexed) e.$2]);
}

/// Two sorted lists of marks as one, sorted by time.
List<LabMark> labMergeMarks(List<LabMark> a, List<LabMark> b) {
  if (b.isEmpty) return List.of(a);
  if (a.isEmpty) return List.of(b);
  final out = <LabMark>[];
  var i = 0;
  var j = 0;
  while (i < a.length && j < b.length) {
    out.add(a[i].time <= b[j].time ? a[i++] : b[j++]);
  }
  while (i < a.length) {
    out.add(a[i++]);
  }
  while (j < b.length) {
    out.add(b[j++]);
  }
  return out;
}
