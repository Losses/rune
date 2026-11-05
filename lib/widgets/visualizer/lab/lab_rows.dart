import 'dart:convert';

import 'package:flutter/foundation.dart';

import 'lab_data.dart';

/// Which strips of which columns the timeline shows, and which rows are
/// pinned to the top. It never changes: every change gives a new selection.
@immutable
class LabRowSelection {
  const LabRowSelection(
    this._map, {
    this.pins = defaultPins,
    this.inputsOn = true,
  });

  /// A column id to whether its plan and its actual strip are on.
  final Map<String, ({bool plan, bool actual})> _map;

  /// The ids of the pinned columns, in the order they are pinned in.
  final List<String> pins;

  /// Whether the scene's inputs that the selection has not been told about
  /// are on. The inputs are the scene's own, so their ids cannot be listed:
  /// a view of every input shows those of the next scene too, and a view
  /// without inputs leaves them out.
  final bool inputsOn;

  /// What is pinned to begin with.
  static const defaultPins = ['sections', 'marks'];

  /// What the lab starts with.
  static const defaults = LabRowSelection({
    'sections': (plan: true, actual: false),
    'marks': (plan: true, actual: true),
    'shot': (plan: true, actual: true),
    'look': (plan: true, actual: true),
    'level': (plan: true, actual: true),
    'drive': (plan: true, actual: true),
    'intensity': (plan: true, actual: true),
    'hit': (plan: true, actual: true),
    'hits': (plan: true, actual: true),
    'low': (plan: true, actual: true),
    'mid': (plan: true, actual: true),
    'high': (plan: true, actual: true),
    'band_low': (plan: false, actual: true),
  });

  /// The ids of the scene's inputs start with this. An input that the
  /// selection has not been told about has both its strips (where it has
  /// them) on when [inputsOn] is, and off otherwise.
  static const inputPrefix = 'input:';

  static bool isInputId(String id) => id.startsWith(inputPrefix);

  // What an id the map does not have is.
  bool _default(String id) => inputsOn && isInputId(id);

  bool planOn(String id) => _map[id]?.plan ?? _default(id);
  bool actualOn(String id) => _map[id]?.actual ?? _default(id);

  /// Whether [column] draws its plan strip: it is on and the column has one.
  bool showsPlan(LabColumn column) =>
      planOn(column.id) && column.plan == LabPlanAvailability.yes;

  /// Whether [column] draws its actual strip.
  bool showsActual(LabColumn column) => actualOn(column.id) && column.hasActual;

  /// Whether any strip of [column] is on: what its own plan and actual
  /// choices show.
  bool isShown(LabColumn column) => showsPlan(column) || showsActual(column);

  LabRowSelection _with({
    Map<String, ({bool plan, bool actual})>? map,
    List<String>? pins,
    bool? inputsOn,
  }) => LabRowSelection(
    map ?? _map,
    pins: pins ?? this.pins,
    inputsOn: inputsOn ?? this.inputsOn,
  );

  LabRowSelection setPart(String id, {bool? plan, bool? actual}) {
    final old = _map[id];
    return _with(
      map: {
        ..._map,
        id: (
          plan: plan ?? old?.plan ?? _default(id),
          actual: actual ?? old?.actual ?? _default(id),
        ),
      },
    );
  }

  /// Every strip that exists on.
  static LabRowSelection all(List<LabColumn> columns) => LabRowSelection({
    for (final c in columns)
      c.id: (plan: c.plan == LabPlanAvailability.yes, actual: c.hasActual),
  });

  /// The same selection with [inputsOn] set.
  LabRowSelection withInputsOn(bool inputsOn) => _with(inputsOn: inputsOn);

  /// Shows [column] with every strip it has, or with none.
  LabRowSelection withVisible(LabColumn column, {required bool on}) => _with(
    map: {
      ..._map,
      column.id: (
        plan: on && column.plan == LabPlanAvailability.yes,
        actual: on && column.hasActual,
      ),
    },
  );

  bool isPinned(String id) => pins.contains(id);

  /// Pins [id] after the others.
  LabRowSelection pin(String id) =>
      isPinned(id) ? this : _with(pins: [...pins, id]);

  LabRowSelection unpin(String id) =>
      isPinned(id) ? _with(pins: [...pins]..remove(id)) : this;

  /// Moves the pin of [id] [by] places, which stops at the ends.
  LabRowSelection movePin(String id, int by) {
    final from = pins.indexOf(id);
    if (from < 0) return this;
    final to = (from + by).clamp(0, pins.length - 1);
    if (to == from) return this;
    final next = [...pins]..removeAt(from);
    next.insert(to, id);
    return _with(pins: next);
  }

  /// The columns that go with [column]: for an input, the columns its
  /// bindings read; for any other column, the inputs that read it.
  static List<LabColumn> related(LabColumn column, List<LabColumn> columns) {
    if (isInputId(column.id)) {
      return [
        for (final c in columns)
          if (column.binds.contains(c.id)) c,
      ];
    }
    return [
      for (final c in columns)
        if (isInputId(c.id) && c.binds.contains(column.id)) c,
    ];
  }

  // The entries that differ from what an id not in the map has, so two
  // selections that show the same are equal.
  Map<String, ({bool plan, bool actual})> get _normal => {
    for (final e in _map.entries)
      if (e.value.plan != _default(e.key) || e.value.actual != _default(e.key))
        e.key: e.value,
  };

  @override
  bool operator ==(Object other) {
    if (other is! LabRowSelection) return false;
    if (inputsOn != other.inputsOn || !listEquals(pins, other.pins)) {
      return false;
    }
    final a = _normal;
    final b = other._normal;
    return a.length == b.length && a.entries.every((e) => b[e.key] == e.value);
  }

  @override
  int get hashCode {
    final entries = _normal.entries.toList()
      ..sort((a, b) => a.key.compareTo(b.key));
    return Object.hash(
      Object.hashAll([for (final e in entries) Object.hash(e.key, e.value)]),
      Object.hashAll(pins),
      inputsOn,
    );
  }

  Map<String, Object?> toMap() => {
    'v': 2,
    'rows': {
      for (final e in _map.entries)
        e.key: {'p': e.value.plan ? 1 : 0, 'a': e.value.actual ? 1 : 0},
    },
    'pins': pins,
    'inputs': inputsOn,
  };

  String toJson() => jsonEncode(toMap());

  /// The selection of a saved [json], or the defaults when it is missing,
  /// cannot be read or is not of the current format.
  static LabRowSelection fromJson(String? json) {
    if (json == null) return defaults;
    try {
      return fromObject(jsonDecode(json)) ?? defaults;
    } catch (_) {
      return defaults;
    }
  }

  /// The selection a decoded JSON value tells, or null when it is not one.
  static LabRowSelection? fromObject(Object? decoded) {
    if (decoded is! Map || decoded['v'] != 2 || decoded['rows'] is! Map) {
      return null;
    }
    final rows = decoded['rows'] as Map;
    final map = <String, ({bool plan, bool actual})>{};
    for (final e in rows.entries) {
      final v = e.value;
      if (e.key is! String || v is! Map) continue;
      map[e.key as String] = (
        plan: v['p'] == 1 || v['p'] == true,
        actual: v['a'] == 1 || v['a'] == true,
      );
    }
    final pins = decoded['pins'];
    return LabRowSelection(
      map,
      pins: pins is List ? [...pins.whereType<String>()] : defaultPins,
      inputsOn: decoded['inputs'] is bool ? decoded['inputs'] as bool : true,
    );
  }
}
