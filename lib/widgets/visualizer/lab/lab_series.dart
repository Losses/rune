import 'dart:math' as math;
import 'dart:typed_data';

import '../../../bindings/bindings.dart';
import 'lab_data.dart';
import 'lab_rows.dart';

// The input classes below carry the hub's generated enum types into the store.

/// One column of the catalogue, as the hub describes it.
class LabSeriesColumn {
  const LabSeriesColumn({
    required this.id,
    required this.name,
    required this.group,
    required this.kind,
    this.lo = 0,
    this.hi = 1,
    this.unit = '',
    this.categories = const [],
    this.plan = VisualizerLabColumnPlan.yes,
    this.binds = const [],
    this.rest,
  });

  final String id;
  final String name;

  /// "Director", "Looks & Level", "Signals" or "Inputs".
  final VisualizerLabColumnGroup group;

  /// "number", "event", "bool", "category" or "phase".
  final VisualizerLabColumnKind kind;
  final double lo;
  final double hi;

  /// Empty when the column has none.
  final String unit;
  final List<String> categories;

  /// "yes" or "no".
  final VisualizerLabColumnPlan plan;

  /// For a scene input, the ids of the columns its bindings read.
  final List<String> binds;

  /// For a scene input, the value it sits at when nothing drives it.
  final double? rest;
}

/// The columns of the series. The version changes when the scene (or its
/// inputs and bindings) does, and for every new song too, as the engine starts
/// a new catalogue with it. Every sample and plan series follows one.
class LabSeriesCatalogue {
  const LabSeriesCatalogue({
    required this.version,
    required this.rate,
    required this.columns,
  });

  final int version;

  /// Buckets in a second of the song.
  final int rate;
  final List<LabSeriesColumn> columns;
}

/// The values of one bucket: one per column of the catalogue the sample
/// follows, NaN where there is none.
class LabSeriesSample {
  const LabSeriesSample({
    required this.bucket,
    required this.columnsVersion,
    required this.values,
  });

  final int bucket;
  final int columnsVersion;
  final List<double> values;
}

/// The plan's series for every column, column-major: the value of column `c`
/// in bucket `b` is `values[c * buckets + b]`.
class LabSeriesPlan {
  const LabSeriesPlan({
    required this.columnsVersion,
    required this.rate,
    required this.buckets,
    required this.values,
  });

  final int columnsVersion;
  final int rate;
  final int buckets;
  final List<double> values;
}

/// What a response of the lab adds to the series: a catalogue when the hub
/// sent a new one, and the buckets that finished since the last response.
class LabSeriesUpdate {
  const LabSeriesUpdate({this.catalogue, this.samples = const []});

  final LabSeriesCatalogue? catalogue;
  final List<LabSeriesSample> samples;
}

/// The columns that are not series: the sections and the marks. They come
/// first in [LabSeriesStore.columns].
const labSectionsColumn = LabColumn(
  id: 'sections',
  name: 'Sections',
  group: 'Plan & Actual',
  kind: LabColumnKind.sections,
  hasActual: false,
);

const labMarksColumn = LabColumn(
  id: 'marks',
  name: 'Marks',
  group: 'Plan & Actual',
  kind: LabColumnKind.marks,
);

/// The kind of a column of the catalogue, as the lab draws it.
LabColumnKind labColumnKindOf(VisualizerLabColumnKind kind) => switch (kind) {
  VisualizerLabColumnKind.number => LabColumnKind.number,
  VisualizerLabColumnKind.event => LabColumnKind.event,
  VisualizerLabColumnKind.bool => LabColumnKind.bool,
  VisualizerLabColumnKind.category => LabColumnKind.category,
  VisualizerLabColumnKind.phase => LabColumnKind.phase,
};

String labColumnGroupName(VisualizerLabColumnGroup group) => switch (group) {
  VisualizerLabColumnGroup.director => 'Director',
  VisualizerLabColumnGroup.looks => 'Looks & Level',
  VisualizerLabColumnGroup.signals => 'Signals',
  VisualizerLabColumnGroup.inputs => 'Inputs',
};

/// Whether the plan has a series for a column.
LabPlanAvailability labPlanAvailabilityOf(VisualizerLabColumnPlan plan) => plan;

// What a column has shown of the song so far, for telling a row with nothing
// in it. It only grows: a value that a later sample writes over is not taken
// back.
class _Seen {
  bool any = false;
  double min = double.infinity;
  double max = double.negativeInfinity;

  void add(double v, _SeenRule rule) {
    if (!v.isFinite) return;
    switch (rule) {
      case _SeenRule.range || _SeenRule.level:
        any = true;
        min = math.min(min, v);
        max = math.max(max, v);
      case _SeenRule.positive:
        if (v > 0) any = true;
      case _SeenRule.finite:
        any = true;
    }
  }

  /// Whether what was seen is worth showing, for a column whose range starts
  /// at [lo].
  bool has(_SeenRule rule, double lo) => switch (rule) {
    _SeenRule.range => any && max > min,
    _SeenRule.level => any && (max > min || min != lo),
    _ => any,
  };
}

// What makes a value of a column worth showing.
enum _SeenRule {
  // The value changes, as a phase or an input does: an input that stays put
  // stays at its rest.
  range,
  // The value changes, or stays somewhere other than the bottom of its range,
  // as a number does: a tempo that holds through the song is worth seeing, a
  // level that stays at nothing is not.
  level,
  // Something is on or has happened, as an event or a bool does.
  positive,
  // Any value, as a category does.
  finite,
}

_SeenRule _ruleOf(LabColumn column) {
  if (LabRowSelection.isInputId(column.id)) return _SeenRule.range;
  return switch (column.kind) {
    LabColumnKind.event || LabColumnKind.bool => _SeenRule.positive,
    LabColumnKind.category => _SeenRule.finite,
    LabColumnKind.number => _SeenRule.level,
    LabColumnKind.phase ||
    LabColumnKind.sections ||
    LabColumnKind.marks => _SeenRule.range,
  };
}

/// What the lab knows of the song's series: the columns, the plan's series
/// and the actual series, recorded as the song plays.
///
/// An actual series is a [Float32List] indexed by bucket, NaN where nothing is
/// recorded; a sample for a bucket that has a value overwrites it, as after a
/// seek back. The store starts afresh ([reset]) for another song, and for
/// another version of the catalogue, whose values mean other columns.
///
/// A column counts as empty only once the store has the plan of the song, or
/// at least [emptyAfter] of recorded buckets: before that, nothing is known.
///
/// The series that [planSeries] gives are the data's: the store does not
/// change them afterwards, it makes new ones. Those of [actualSeries] are the
/// store's own buffers, which it writes into as samples come in: [revision]
/// changes whenever they do, and whoever draws them compares it.
class LabSeriesStore {
  int _version = 0;
  int _rate = labBucketRate;
  List<LabSeriesColumn> _source = const [];
  List<LabColumn> _columns = const [labSectionsColumn, labMarksColumn];

  // The recorded values by column index, each as long as the capacity.
  List<Float32List> _actual = const [];
  int _capacity = 0;
  // The most buckets the song has, when it is known; none grows as needed.
  // It is the larger of what the plan has and what [setBuckets] was told.
  int _limit = 0;
  int _planBuckets = 0;
  int _known = 0;

  Map<String, Float32List> _plan = const {};
  // The buffers by column id, made again only when they are replaced.
  Map<String, Float32List>? _actualMap = const {};
  int _revision = 0;

  // What each column of the catalogue has shown, by column index, and what
  // makes a value of it count.
  List<_Seen> _seen = const [];
  List<_SeenRule> _rules = const [];
  List<double> _lows = const [];
  // The ids of the columns the plan has something in, and whether the store
  // has the plan of the song.
  Set<String> _planHas = const {};
  bool _planKnown = false;
  // How many samples were recorded: after a seek the buckets before the
  // first sample are not what was heard.
  int _recorded = 0;
  Set<String>? _emptyColumns = const {};
  // The set [emptyColumns] gave last, kept while what is in it stays.
  Set<String> _lastEmpty = const {};

  /// How much recorded song a column needs to be called empty when the store
  /// has no plan.
  static const emptyAfter = Duration(seconds: 10);

  /// The catalogue's version, 0 while there is none.
  int get version => _version;

  /// The sections and the marks, then the columns of the catalogue.
  List<LabColumn> get columns => _columns;

  /// By column id; the columns the plan has no series for are not in it.
  Map<String, Float32List> get planSeries => _plan;

  /// By column id, a series as long as the buckets known. These are the
  /// buffers the samples are written into, not copies: the map is the same
  /// until the buffers are replaced, and [revision] tells that a value in them
  /// changed.
  Map<String, Float32List> get actualSeries {
    return _actualMap ??= {
      for (var i = 0; i < _source.length; i++) _source[i].id: _actual[i],
    };
  }

  /// Changes whenever [actualSeries] does, in its values or in the buffers.
  int get revision => _revision;

  /// The ids of the columns that have nothing to show in this song: a number
  /// that stays at the bottom of its range, a phase or an input that never
  /// changes, no event, nothing on in a bool, no value in a category. The
  /// sections and the marks always have something. It is empty until the plan
  /// is in or [emptyAfter] is recorded, and it is the same set until it
  /// changes.
  Set<String> get emptyColumns => _emptyColumns ??= _emptySet();

  Set<String> _emptySet() {
    final Set<String> next;
    if (!_planKnown && _recorded < emptyAfter.inSeconds * _rate) {
      next = const {};
    } else {
      next = {
        for (var i = 0; i < _source.length; i++)
          if (!_seen[i].has(_rules[i], _lows[i]) &&
              !_planHas.contains(_source[i].id))
            _source[i].id,
      };
    }
    if (next.length == _lastEmpty.length && next.containsAll(_lastEmpty)) {
      return _lastEmpty;
    }
    return _lastEmpty = next;
  }

  /// Tells how many buckets the song has by the player's length, which the
  /// series are as long as: the larger of it and the plan's. Buckets that were
  /// recorded past it are dropped. It is for a song whose plan has no series,
  /// or one whose length the player gives as longer than the chart.
  void setBuckets(int buckets) {
    if (buckets < 0 || buckets == _known) return;
    _known = buckets;
    _applyLimit();
  }

  // The limit is the larger of the plan's buckets and the known ones.
  void _applyLimit() {
    _limit = math.max(_planBuckets, _known);
    _resize(_limit);
  }

  /// Forgets everything of the song: the catalogue, the plan's series and what
  /// was recorded.
  void reset() {
    _version = 0;
    _source = const [];
    _columns = const [labSectionsColumn, labMarksColumn];
    _actual = const [];
    _capacity = 0;
    _limit = 0;
    _planBuckets = 0;
    _known = 0;
    _plan = const {};
    _actualMap = const {};
    _revision++;
    _seen = const [];
    _rules = const [];
    _lows = const [];
    _planHas = const {};
    _planKnown = false;
    _recorded = 0;
    _emptyColumns = const {};
    _lastEmpty = const {};
  }

  /// Takes in a catalogue. One of the version the store has already changes
  /// nothing; another version starts the store afresh.
  void setCatalogue(LabSeriesCatalogue catalogue) {
    if (catalogue.version == _version && _version != 0) return;
    final planBuckets = _planBuckets;
    final known = _known;
    reset();
    _planBuckets = planBuckets;
    _known = known;
    _limit = math.max(planBuckets, known);
    _version = catalogue.version;
    _rate = catalogue.rate > 0 ? catalogue.rate : labBucketRate;
    _source = List.unmodifiable(catalogue.columns);
    _columns = List.unmodifiable([
      labSectionsColumn,
      labMarksColumn,
      for (final c in _source)
        LabColumn(
          id: c.id,
          name: c.name,
          group: labColumnGroupName(c.group),
          kind: labColumnKindOf(c.kind),
          lo: c.lo,
          hi: c.hi,
          unit: c.unit.isEmpty ? null : c.unit,
          categories: c.categories,
          plan: labPlanAvailabilityOf(c.plan),
          binds: c.binds,
          rest: c.rest,
        ),
    ]);
    _actual = [for (final _ in _source) Float32List(0)];
    _actualMap = null;
    _seen = [for (final _ in _source) _Seen()];
    _rules = [for (final c in _columns.skip(2)) _ruleOf(c)];
    _lows = [for (final c in _columns.skip(2)) c.lo];
    _emptyColumns = null;
    _resize(_limit);
  }

  /// Records [samples]: the values go to their buckets, over any that are
  /// there. A sample of another catalogue than the store's is left out.
  void addSamples(Iterable<LabSeriesSample> samples) {
    if (_version == 0) return;
    var changed = false;
    for (final s in samples) {
      if (s.columnsVersion != _version || s.bucket < 0) continue;
      if (_limit > 0 && s.bucket >= _limit) continue;
      if (s.bucket >= _capacity) _resize(_grownCapacity(s.bucket + 1));
      final n = s.values.length < _actual.length
          ? s.values.length
          : _actual.length;
      for (var c = 0; c < n; c++) {
        final v = s.values[c];
        _actual[c][s.bucket] = v;
        _seen[c].add(v, _rules[c]);
      }
      // A column the sample has no value for has none for the bucket.
      for (var c = n; c < _actual.length; c++) {
        _actual[c][s.bucket] = double.nan;
      }
      _recorded++;
      changed = true;
    }
    if (changed) {
      _revision++;
      _emptyColumns = null;
    }
  }

  /// Takes in the plan's series. A plan of another catalogue than the store's
  /// is left out, and it is whether the store took it that is returned.
  bool importPlan(LabSeriesPlan plan) {
    if (_version == 0 || plan.columnsVersion != _version) return false;
    final buckets = plan.buckets;
    if (buckets < 0 || plan.values.length < _source.length * buckets) {
      return false;
    }
    final series = <String, Float32List>{};
    final has = <String>{};
    for (var c = 0; c < _source.length; c++) {
      if (labPlanAvailabilityOf(_source[c].plan) != LabPlanAvailability.yes) {
        continue;
      }
      final out = Float32List(buckets);
      final from = c * buckets;
      final seen = _Seen();
      for (var b = 0; b < buckets; b++) {
        out[b] = plan.values[from + b];
        seen.add(out[b], _rules[c]);
      }
      series[_source[c].id] = out;
      if (seen.has(_rules[c], _lows[c])) has.add(_source[c].id);
    }
    _plan = series;
    _planHas = has;
    _planKnown = true;
    _emptyColumns = null;
    _planBuckets = buckets;
    _applyLimit();
    return true;
  }

  /// Drops the plan's series (the plan went away).
  void clearPlan() {
    _planBuckets = 0;
    _plan = const {};
    _planHas = const {};
    _planKnown = false;
    _emptyColumns = null;
  }

  int _grownCapacity(int need) {
    var n = _capacity < 64 ? 64 : _capacity;
    while (n < need) {
      n *= 2;
    }
    return n;
  }

  // Makes every series [size] long, keeping the values that still fit.
  void _resize(int size) {
    if (size == _capacity) return;
    _actual = [
      for (final old in _actual)
        Float32List(size)
          ..fillRange(0, size, double.nan)
          ..setRange(0, old.length < size ? old.length : size, old),
    ];
    _capacity = size;
    _actualMap = null;
    _revision++;
  }
}
