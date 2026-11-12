import 'dart:math' as math;
import 'dart:typed_data';
import 'dart:ui' show Color;

import 'lab_data.dart';

/// The shortest stretch of the song the timeline zooms to, in seconds.
const labMinSpan = 4.0;

/// The part of the song the timeline shows.
class LabView {
  const LabView(this.start, this.end);

  /// The whole song.
  factory LabView.whole(double length) => LabView(0, length);

  final double start;
  final double end;

  double get span => end - start;

  bool isWhole(double length) => span >= length - 1e-6;

  @override
  bool operator ==(Object other) =>
      other is LabView && other.start == start && other.end == end;

  @override
  int get hashCode => Object.hash(start, end);
}

double _clamp(double v, double lo, double hi) => math.max(lo, math.min(hi, v));

/// [view] moved to lie within a song of [length], keeping its span as far as
/// it fits.
LabView labClampView(LabView view, double length) {
  final span = math.min(
    math.max(view.span, math.min(labMinSpan, length)),
    length,
  );
  final start = _clamp(view.start, 0, length - span);
  return LabView(start, start + span);
}

/// [view] zoomed by [factor] (above 1 shows more), so that the song time
/// [center] stays where it is on the screen. The span stays between
/// [labMinSpan] and the whole song.
LabView labZoomView(LabView view, double length, double center, double factor) {
  final span = view.span;
  final minSpan = math.min(labMinSpan, length);
  final next = _clamp(span * factor, minSpan, length);
  final ratio = span > 0 ? (center - view.start) / span : 0.5;
  final start = _clamp(center - ratio * next, 0, length - next);
  return LabView(start, start + next);
}

/// [view] moved by [delta] seconds, within the song. A view of the whole song
/// does not move.
LabView labPanView(LabView view, double length, double delta) {
  final span = view.span;
  if (span >= length - 1e-6) return view;
  final start = _clamp(view.start + delta, 0, length - span);
  return LabView(start, start + span);
}

/// The view that keeps [time] at 35% of its width.
LabView labFollowView(LabView view, double length, double time) {
  final span = view.span;
  final start = _clamp(time - span * 0.35, 0, length - span);
  return LabView(start, start + span);
}

/// The song time the timeline draws at each frame. The lab reads where the
/// song is about ten times a second; between two readings the clock runs on
/// by itself while the song plays, so that the playhead and a followed view
/// move at every frame rather than in steps. A reading near the clock steers
/// its speed, so that it never runs back; a pause, another song or a reading
/// far from it (a seek) sets it at once.
class LabPlayClock {
  /// The farthest a reading may be from the clock and still only steer it.
  static const steerWithin = 0.5;

  /// How long the clock takes to make up the difference to a reading.
  static const settle = 0.4;

  /// The most the clock runs faster or slower than the song while it does.
  static const maxSkew = 0.3;

  /// The longest the clock runs on past a reading with no other one.
  static const maxLead = 0.5;

  double _time = 0;
  double _at = 0;
  double _rate = 1;
  bool _playing = false;

  /// The time on the clock at [wall], in seconds of a steady clock.
  double timeAt(double wall) {
    if (!_playing) return _time;
    final elapsed = _clamp(wall - _at, 0, maxLead);
    return _time + _rate * elapsed;
  }

  /// A reading of the song: at [time], playing or not, taken at [wall].
  /// With [jump] the clock is set to it whatever it showed.
  void read(
    double time, {
    required bool playing,
    required double wall,
    bool jump = false,
  }) {
    final shown = timeAt(wall);
    final gap = time - shown;
    if (jump || !playing || !_playing || gap.abs() > steerWithin) {
      _time = time;
      _rate = 1;
    } else {
      _time = shown;
      _rate = _clamp(1 + gap / settle, 1 - maxSkew, 1 + maxSkew);
    }
    _at = wall;
    _playing = playing;
  }
}

/// The value a pixel column shows for the buckets [b0] to [b1) (the end
/// excluded) of a series of [kind]: the largest of numbers, the sum of
/// events, the last of the others. NaN when none was recorded.
double labAggregate(LabColumnKind kind, Float32List series, int b0, int b1) {
  var out = double.nan;
  for (var b = b0; b < b1; b++) {
    final v = series[b];
    if (v.isNaN) continue;
    if (out.isNaN) {
      out = v;
    } else if (kind == LabColumnKind.number) {
      if (v > out) out = v;
    } else if (kind == LabColumnKind.event) {
      out += v;
    } else {
      out = v;
    }
  }
  return out;
}

/// The first index of [list] whose [time] is not below [x].
int labLowerBound<T>(List<T> list, double x, double Function(T) time) {
  var lo = 0;
  var hi = list.length;
  while (lo < hi) {
    final mid = (lo + hi) >> 1;
    if (time(list[mid]) < x) {
      lo = mid + 1;
    } else {
      hi = mid;
    }
  }
  return lo;
}

// ---------- colors ----------

const _rampStops = <(double, int, int, int)>[
  (0, 14, 34, 48),
  (0.4, 19, 96, 126),
  (0.75, 56, 191, 224),
  (1, 230, 251, 255),
];

/// 64 steps from dark navy through teal and cyan to near-white.
final List<Color> labRamp = List.unmodifiable(() {
  final out = <Color>[];
  for (var i = 0; i < 64; i++) {
    final v = i / 63;
    var k = 0;
    while (k < _rampStops.length - 2 && v > _rampStops[k + 1].$1) {
      k++;
    }
    final a = _rampStops[k];
    final b = _rampStops[k + 1];
    final u = (v - a.$1) / (b.$1 - a.$1);
    int mix(int x, int y) => (x + (y - x) * u).round();
    out.add(
      Color.fromARGB(255, mix(a.$2, b.$2), mix(a.$3, b.$3), mix(a.$4, b.$4)),
    );
  }
  return out;
}());

/// The colors of category values that a column does not give its own.
const labCategoryPalette = <Color>[
  Color(0xFF5B8DEF),
  Color(0xFF2FB8A6),
  Color(0xFFB07BE0),
  Color(0xFFE3A34B),
  Color(0xFFE0677F),
  Color(0xFF7FC46A),
  Color(0xFF8A94A8),
  Color(0xFFD6C58A),
];

const _coolLow = (60, 140, 235);
const _coolHigh = (150, 205, 255);
const _warmLow = (235, 120, 50);
const _warmHigh = (255, 205, 120);

List<Color> _restRamp((int, int, int) a, (int, int, int) b) =>
    List.unmodifiable([
      for (var i = 0; i < 64; i++)
        () {
          final s = i / 63;
          int mix(int x, int y) => (x + (y - x) * s).round();
          // Near the rest value it is almost transparent.
          return Color.fromARGB(
            (255 * (0.03 + 0.97 * s)).round(),
            mix(a.$1, b.$1),
            mix(a.$2, b.$2),
            mix(a.$3, b.$3),
          );
        }(),
    ]);

/// 64 steps of the color below a rest value, from almost transparent at it
/// to a bright blue far under it.
final List<Color> labCoolRamp = _restRamp(_coolLow, _coolHigh);

/// 64 steps of the color above a rest value, from almost transparent at it
/// to a bright amber far over it.
final List<Color> labWarmRamp = _restRamp(_warmLow, _warmHigh);

/// The color [column] draws [value] in when it is drawn around its rest
/// value: cool below it, warm above, in the strength of the distance to the
/// end of the range. Null for a NaN.
Color? labRestColor(LabColumn column, double value) {
  if (value.isNaN) return null;
  final rest = column.rest ?? column.lo;
  final below = value < rest;
  final room = below ? rest - column.lo : column.hi - rest;
  final strength = room <= 0
      ? 0.0
      : _clamp((below ? rest - value : value - rest) / room, 0, 1);
  return (below ? labCoolRamp : labWarmRamp)[(strength * 63).round()];
}

/// How bright a pixel column of ticks is when [sum] events fall in it: none
/// is nothing, and many do not make a solid bar.
double labDensityAlpha(double sum) =>
    sum > 0 ? 0.25 + 0.75 * (1 - math.exp(-sum / 2)) : 0;

const labBoolOn = Color(0xFFE0B25A);
const _boolOff = Color(0x0DFFFFFF);
const _eventColors = <Color>[
  Color(0x8CFFFFFF),
  Color(0xCCFFFFFF),
  Color(0xFFFFFFFF),
];

/// The color of an event count; the color for 1, 2 and more.
Color labEventColor(double count) =>
    _eventColors[math.min(3, math.max(1, count.round())) - 1];

/// The color [column] draws [value] in, or null for nothing (a NaN, or no
/// event).
Color? labColorFor(LabColumn column, double value) {
  if (value.isNaN) return null;
  switch (column.kind) {
    case LabColumnKind.number:
      final range = column.hi - column.lo;
      final f = range == 0 ? 0.0 : (value - column.lo) / range;
      return labRamp[_clamp(f * 63, 0, 63).round()];
    case LabColumnKind.phase:
      return labRamp[_clamp(value * 63, 0, 63).round()];
    case LabColumnKind.bool:
      return value >= 0.5 ? labBoolOn : _boolOff;
    case LabColumnKind.category:
      final palette = labCategoryPalette;
      if (palette.isEmpty) return null;
      final i = value.toInt() % palette.length;
      return palette[i < 0 ? i + palette.length : i];
    case LabColumnKind.event:
      return value > 0 ? labEventColor(value) : null;
    case LabColumnKind.sections:
    case LabColumnKind.marks:
      return null;
  }
}

/// A value as the tooltip and the label show it.
String labFormatValue(LabColumn column, double value) {
  if (value.isNaN) return '—';
  switch (column.kind) {
    case LabColumnKind.category:
      final i = value.toInt();
      return i >= 0 && i < column.categories.length
          ? column.categories[i]
          : '—';
    case LabColumnKind.bool:
      return value >= 0.5 ? 'yes' : 'no';
    case LabColumnKind.event:
      return value.toInt().toString();
    case LabColumnKind.number:
    case LabColumnKind.phase:
    case LabColumnKind.sections:
    case LabColumnKind.marks:
      return switch (column.unit) {
        'bpm' => '${value.toStringAsFixed(0)} bpm',
        's' => '${value.toStringAsFixed(1)} s',
        _ => value.toStringAsFixed(2),
      };
  }
}

/// A time as minutes and seconds, with tenths when asked.
String labFormatTime(double t, {bool tenths = false}) {
  t = math.max(0, t);
  final m = t ~/ 60;
  final s = t - m * 60;
  if (tenths) {
    // The tenths round down, so that 59.96 s is not 0:60.0.
    final whole = (t * 10).floor();
    return '${whole ~/ 600}:${((whole % 600) / 10).toStringAsFixed(1).padLeft(4, '0')}';
  }
  return '$m:${s.floor().toString().padLeft(2, '0')}';
}
