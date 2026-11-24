import 'lab_data.dart';

/// How a row of the timeline draws its values. Every column has its own
/// ([labDefaultStyle]); a color band can be expanded to a line.
enum LabDrawStyle {
  /// A color for each value, from the ramp.
  band,

  /// A color for the distance from the rest value: cool below, warm above.
  aroundRest,

  /// A line through the values.
  line,

  /// A line that steps from one category to the next.
  steps,

  /// A block for each run of a category, with its name.
  blocks,

  /// A block for each run of a value, with the value.
  numbers,

  /// A tick for each event.
  ticks,

  /// A bar while the value is on.
  fill,
}

bool _restInside(LabColumn c) {
  final rest = c.rest;
  return rest != null && rest > c.lo && rest < c.hi;
}

/// How [c] is drawn when the selection does not say.
LabDrawStyle labDefaultStyle(LabColumn c) {
  switch (c.kind) {
    case LabColumnKind.category:
      return switch (c.id) {
        'size' || 'tier' || 'pace' || 'level' => LabDrawStyle.steps,
        _ => LabDrawStyle.blocks,
      };
    case LabColumnKind.number:
      return switch (c.id) {
        'tempo' ||
        'meter' ||
        'section' ||
        'section_label' => LabDrawStyle.numbers,
        'cue_in' || 'age' => LabDrawStyle.line,
        _ when _restInside(c) => LabDrawStyle.aroundRest,
        _ => LabDrawStyle.band,
      };
    case LabColumnKind.phase:
      return LabDrawStyle.line;
    case LabColumnKind.event:
      return LabDrawStyle.ticks;
    case LabColumnKind.bool:
      return LabDrawStyle.fill;
    case LabColumnKind.sections:
    case LabColumnKind.marks:
      return LabDrawStyle.band;
  }
}

/// Whether a row of [c] can be expanded from its color band to a line: a
/// level, where the band shows at a glance how much and the line how the plan
/// and the actual differ.
bool labExpandable(LabColumn c) => switch (labDefaultStyle(c)) {
  LabDrawStyle.band ||
  LabDrawStyle.aroundRest => c.kind == LabColumnKind.number,
  _ => false,
};
