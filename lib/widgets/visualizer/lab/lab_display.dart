import 'dart:convert';

import 'package:flutter/foundation.dart';

import 'lab_data.dart';
import 'lab_draw_style.dart';
import 'lab_rows.dart';

/// Which strips the timeline draws on every row.
enum LabStripMode { plan, actual, both }

/// How the timeline draws the rows a [LabRowSelection] shows: which strips,
/// which levels as lines, and whether rows with nothing in the song are left
/// out. It is not part of a view, so switching views keeps it. It never
/// changes: every change gives a new display.
@immutable
class LabDisplay {
  const LabDisplay({
    this.stripMode = LabStripMode.both,
    this.allLines = false,
    this.expanded = const {},
    this.hideEmpty = true,
  });

  /// Which strips every shown row draws. It does not change the strips each
  /// row has on: [LabStripMode.plan] draws the plan of a row that has one,
  /// else its actual; [LabStripMode.actual] the other way round.
  final LabStripMode stripMode;

  /// Whether every level is drawn as a line; [expanded] then holds the levels
  /// folded back to their color band.
  final bool allLines;

  /// The ids of the levels turned the other way from [allLines] one by one.
  final Set<String> expanded;

  /// Whether the timeline leaves out rows with nothing in this song.
  final bool hideEmpty;

  /// The same display with [stripMode] set.
  LabDisplay withStripMode(LabStripMode mode) => mode == stripMode
      ? this
      : LabDisplay(
          stripMode: mode,
          allLines: allLines,
          expanded: expanded,
          hideEmpty: hideEmpty,
        );

  /// The same display with every level a line, or every level its band; the
  /// levels that were turned the other way one by one are put back.
  LabDisplay withAllLines(bool on) =>
      LabDisplay(stripMode: stripMode, allLines: on, hideEmpty: hideEmpty);

  /// The same display with [hideEmpty] set.
  LabDisplay withHideEmpty(bool on) => on == hideEmpty
      ? this
      : LabDisplay(
          stripMode: stripMode,
          allLines: allLines,
          expanded: expanded,
          hideEmpty: on,
        );

  /// Whether [id] is turned the other way from [allLines]: a line when
  /// [allLines] is off, its color band when it is on.
  bool isExpanded(String id) => expanded.contains(id);

  /// The same display with [id] turned, or not, from [allLines].
  LabDisplay withExpanded(String id, {required bool on}) => on == isExpanded(id)
      ? this
      : LabDisplay(
          stripMode: stripMode,
          allLines: allLines,
          hideEmpty: hideEmpty,
          expanded: on
              ? {...expanded, id}
              : {
                  for (final e in expanded)
                    if (e != id) e,
                },
        );

  /// The style [column] is drawn in: for a level, a line when [allLines]
  /// differs from the level being in [expanded]; else its own.
  LabDrawStyle styleOf(LabColumn column) =>
      labExpandable(column) && allLines != expanded.contains(column.id)
      ? LabDrawStyle.line
      : labDefaultStyle(column);

  bool _hasPlan(LabColumn column) => column.plan == LabPlanAvailability.yes;

  /// Whether [column] draws its plan strip. With both strips, as the row
  /// says; on plan, every shown row that has a plan; on actual, a shown row
  /// that has no actual to draw instead.
  bool showsPlan(LabRowSelection selection, LabColumn column) =>
      switch (stripMode) {
        LabStripMode.both => selection.showsPlan(column),
        LabStripMode.plan => selection.isShown(column) && _hasPlan(column),
        LabStripMode.actual =>
          selection.isShown(column) && !column.hasActual && _hasPlan(column),
      };

  /// Whether [column] draws its actual strip, the other way round from
  /// [showsPlan].
  bool showsActual(LabRowSelection selection, LabColumn column) =>
      switch (stripMode) {
        LabStripMode.both => selection.showsActual(column),
        LabStripMode.actual => selection.isShown(column) && column.hasActual,
        LabStripMode.plan =>
          selection.isShown(column) && !_hasPlan(column) && column.hasActual,
      };

  @override
  bool operator ==(Object other) =>
      other is LabDisplay &&
      stripMode == other.stripMode &&
      allLines == other.allLines &&
      hideEmpty == other.hideEmpty &&
      setEquals(expanded, other.expanded);

  @override
  int get hashCode {
    final lines = expanded.toList()..sort();
    return Object.hash(stripMode, allLines, hideEmpty, Object.hashAll(lines));
  }

  Map<String, Object?> toMap() => {
    'strips': stripMode.name,
    'lines': allLines,
    'expanded': expanded.toList()..sort(),
    'hideEmpty': hideEmpty,
  };

  String toJson() => jsonEncode(toMap());

  /// The display of a saved [json], or the defaults when it is missing or
  /// cannot be read.
  static LabDisplay fromJson(String? json) {
    if (json == null) return const LabDisplay();
    try {
      return fromObject(jsonDecode(json)) ?? const LabDisplay();
    } catch (_) {
      return const LabDisplay();
    }
  }

  /// The display a decoded JSON value tells, or null when it is not a map.
  static LabDisplay? fromObject(Object? decoded) {
    if (decoded is! Map) return null;
    final saved = decoded['expanded'];
    return LabDisplay(
      stripMode: switch (decoded['strips']) {
        'plan' => LabStripMode.plan,
        'actual' => LabStripMode.actual,
        _ => LabStripMode.both,
      },
      allLines: decoded['lines'] == true,
      expanded: {if (saved is List) ...saved.whereType<String>()},
      hideEmpty: decoded['hideEmpty'] is bool
          ? decoded['hideEmpty'] as bool
          : true,
    );
  }
}
