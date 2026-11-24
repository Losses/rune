import 'dart:convert';

import 'package:flutter/foundation.dart';

import 'lab_data.dart';
import 'lab_display.dart';
import 'lab_rows.dart';

/// The name of the view the lab starts with.
const labOverviewName = 'Overview';

/// A named set of rows to show.
@immutable
class LabRowView {
  const LabRowView({
    required this.name,
    required this.selection,
    this.builtIn = false,
  });

  final String name;

  /// Whether the view comes with the lab: it cannot be renamed, updated or
  /// deleted.
  final bool builtIn;
  final LabRowSelection selection;
}

// The columns each built-in view shows, by id. The ones the catalogue does
// not have are left out.
const _directorIds = [
  'sections',
  'marks',
  'shot',
  'size',
  'tier',
  'pace',
  'age',
  'held',
  'look',
  'mix',
  'look_held',
  'level',
  'cut',
  'moment.strong',
  'moment.medium',
];

const _musicIds = [
  'sections',
  'tempo',
  'meter',
  'drive',
  'intensity',
  'section_level',
  'section_energy',
  'next_energy',
  'hits',
  'hit',
  'beats',
  'bars',
  'cues',
  'cue',
  'impacts',
  'impact',
  'notes',
  'note',
  'low',
  'mid',
  'high',
  'band_low',
  'band_mid',
  'band_high',
];

// The selection that shows [ids] with every strip they have, and nothing
// else. The columns it leaves out are written down as off; the inputs of
// another scene are on only when [inputsOn] is.
LabRowSelection _showing(
  List<LabColumn> columns,
  Set<String> ids, {
  bool inputsOn = false,
}) => LabRowSelection({
  for (final c in columns)
    c.id: (
      plan: ids.contains(c.id) && c.plan == LabPlanAvailability.yes,
      actual: ids.contains(c.id) && c.hasActual,
    ),
}, inputsOn: inputsOn);

// The built-in views made for the columns last asked about: the columns are
// the same list until the catalogue changes, and the views are asked for on
// every build of the timeline.
List<LabColumn>? _builtInColumns;
List<LabRowView> _builtInViews = const [];

/// The views that come with the lab, made for the current [columns]. They are
/// made again only for another list of columns.
List<LabRowView> labBuiltInViews(List<LabColumn> columns) {
  if (identical(columns, _builtInColumns)) return _builtInViews;
  _builtInViews = List.unmodifiable(_makeBuiltInViews(columns));
  _builtInColumns = columns;
  return _builtInViews;
}

List<LabRowView> _makeBuiltInViews(List<LabColumn> columns) {
  final inputs = [
    for (final c in columns)
      if (LabRowSelection.isInputId(c.id)) c,
  ];
  final inputIds = {
    'sections',
    'marks',
    for (final c in inputs) ...[c.id, ...c.binds],
  };
  return [
    const LabRowView(
      name: labOverviewName,
      selection: LabRowSelection.defaults,
      builtIn: true,
    ),
    LabRowView(
      name: 'Director',
      selection: _showing(columns, {..._directorIds}),
      builtIn: true,
    ),
    LabRowView(
      name: 'Music',
      selection: _showing(columns, {..._musicIds}),
      builtIn: true,
    ),
    LabRowView(
      name: 'Inputs',
      selection: _showing(columns, inputIds, inputsOn: true),
      builtIn: true,
    ),
    LabRowView(
      name: 'All',
      selection: LabRowSelection.all(columns),
      builtIn: true,
    ),
  ];
}

/// The names of the built-in views.
const labBuiltInNames = ['Overview', 'Director', 'Music', 'Inputs', 'All'];

String _key(String name) => name.trim().toLowerCase();

/// Why [name] cannot be the name of a view, or null when it can. [taken] are
/// the names that are in use; [except] is the view that is being renamed.
String? labViewNameError(
  String name,
  Iterable<String> taken, {
  String? except,
}) {
  final key = _key(name);
  if (key.isEmpty) return 'Enter a name.';
  for (final t in taken) {
    if (except != null && _key(t) == _key(except)) continue;
    if (_key(t) == key) return 'A view with this name exists.';
  }
  return null;
}

/// The views the user saved, as the JSON the setting holds.
String labViewsToJson(List<LabRowView> views) => jsonEncode([
  for (final v in views) {'name': v.name, 'selection': v.selection.toMap()},
]);

/// The views of a saved [json]. An entry that cannot be read, or whose name
/// is empty or taken by an earlier one or a built-in view, is skipped.
List<LabRowView> labViewsFromJson(String? json) {
  if (json == null) return const [];
  try {
    final decoded = jsonDecode(json);
    if (decoded is! List) return const [];
    final out = <LabRowView>[];
    final names = {for (final n in labBuiltInNames) _key(n)};
    for (final e in decoded) {
      if (e is! Map || e['name'] is! String) continue;
      final name = (e['name'] as String).trim();
      final selection = LabRowSelection.fromObject(e['selection']);
      if (selection == null || name.isEmpty || !names.add(_key(name))) continue;
      out.add(LabRowView(name: name, selection: selection));
    }
    return out;
  } catch (_) {
    return const [];
  }
}

/// How the model writes what it holds down.
typedef LabRowsSaver = void Function(String key, String value);

/// The rows the timeline shows, the views the user saved and the one that
/// is current. Changes are written through [onSave]; whoever listens sees
/// the new rows at once.
class LabRowsModel extends ChangeNotifier {
  LabRowsModel({LabRowsSaver? onSave}) : _save = onSave;

  final LabRowsSaver? _save;

  /// The keys [onSave] is called with.
  static const rowsKey = 'rows';
  static const viewsKey = 'views';
  static const viewKey = 'view';
  static const displayKey = 'display';

  LabRowSelection _selection = LabRowSelection.defaults;
  LabDisplay _display = const LabDisplay();
  List<LabRowView> _own = const [];
  String _name = labOverviewName;

  /// The rows the timeline shows.
  LabRowSelection get selection => _selection;

  /// How the rows are drawn. It belongs to no view: switching views keeps it.
  LabDisplay get display => _display;

  /// The name of the current view.
  String get currentName => _name;

  /// The views the user saved.
  List<LabRowView> get ownViews => _own;

  /// Sets what was saved, without writing it back. A value that is missing or
  /// that cannot be read gives the default.
  void load({String? rows, String? views, String? view, String? display}) {
    _selection = LabRowSelection.fromJson(rows);
    _display = LabDisplay.fromJson(display);
    _own = labViewsFromJson(views);
    _name = view ?? labOverviewName;
    notifyListeners();
  }

  /// The built-in views and then the user's, for [columns].
  List<LabRowView> views(List<LabColumn> columns) => [
    ...labBuiltInViews(columns),
    ..._own,
  ];

  /// The current view, or null when it is gone.
  LabRowView? currentView(List<LabColumn> columns) {
    for (final v in views(columns)) {
      if (v.name == _name) return v;
    }
    return null;
  }

  /// Whether the rows differ from what the current view gives.
  bool isEdited(List<LabColumn> columns) =>
      currentView(columns)?.selection != _selection;

  void _write(String key, String value) => _save?.call(key, value);

  void setSelection(LabRowSelection selection) {
    if (selection == _selection) return;
    _selection = selection;
    _write(rowsKey, selection.toJson());
    notifyListeners();
  }

  void setDisplay(LabDisplay display) {
    if (display == _display) return;
    _display = display;
    _write(displayKey, display.toJson());
    notifyListeners();
  }

  void _setName(String name) {
    _name = name;
    _write(viewKey, name);
  }

  void _setOwn(List<LabRowView> own) {
    _own = List.unmodifiable(own);
    _write(viewsKey, labViewsToJson(own));
  }

  /// Shows the rows of the view [name], and makes it current.
  void applyView(String name, List<LabColumn> columns) {
    for (final v in views(columns)) {
      if (v.name != name) continue;
      _selection = v.selection;
      _write(rowsKey, _selection.toJson());
      _setName(v.name);
      notifyListeners();
      return;
    }
  }

  /// Saves the rows as a view of the user's, which becomes current. The
  /// name must pass [labViewNameError].
  void saveView(String name, List<LabColumn> columns) {
    final trimmed = name.trim();
    if (labViewNameError(trimmed, views(columns).map((v) => v.name)) != null) {
      return;
    }
    _setOwn([..._own, LabRowView(name: trimmed, selection: _selection)]);
    _setName(trimmed);
    notifyListeners();
  }

  /// Renames a view of the user's.
  void renameView(String from, String to, List<LabColumn> columns) {
    final trimmed = to.trim();
    final i = _own.indexWhere((v) => v.name == from);
    if (i < 0 ||
        labViewNameError(
              trimmed,
              views(columns).map((v) => v.name),
              except: from,
            ) !=
            null) {
      return;
    }
    final next = [..._own];
    next[i] = LabRowView(name: trimmed, selection: next[i].selection);
    _setOwn(next);
    if (_name == from) _setName(trimmed);
    notifyListeners();
  }

  /// Replaces the rows of a view of the user's with the current ones.
  void updateView(String name) {
    final i = _own.indexWhere((v) => v.name == name);
    if (i < 0) return;
    final next = [..._own];
    next[i] = LabRowView(name: name, selection: _selection);
    _setOwn(next);
    notifyListeners();
  }

  /// Deletes a view of the user's. If it was current, the rows stay and the
  /// view falls back to the overview, which they then differ from.
  void deleteView(String name) {
    if (!_own.any((v) => v.name == name)) return;
    _setOwn([
      for (final v in _own)
        if (v.name != name) v,
    ]);
    if (_name == name) _setName(labOverviewName);
    notifyListeners();
  }

  /// The catalogue went from the columns [from] to [to]. A built-in view that
  /// shows what it gave for [from] is made again for [to], as the columns it
  /// lists belong to the scene. A view of the user's, or a built-in view that
  /// was edited, stays as it is.
  void catalogueChanged(List<LabColumn> from, List<LabColumn> to) {
    if (identical(from, to)) return;
    for (final v in labBuiltInViews(from)) {
      if (v.name != _name) continue;
      if (v.selection != _selection) return;
      for (final next in labBuiltInViews(to)) {
        if (next.name != _name || next.selection == _selection) continue;
        applyView(_name, to);
      }
      return;
    }
  }

  /// Shows the rows of the current view again.
  void resetView(List<LabColumn> columns) {
    final view = currentView(columns);
    if (view == null) {
      applyView(labOverviewName, columns);
    } else {
      applyView(view.name, columns);
    }
  }
}
