import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../../../bindings/bindings.dart';
import '../../api/file_dialog.dart';
import '../../settings_manager.dart';

import 'file_dialog.dart';
import 'file_dialog_model.dart';

const _viewKey = 'file_dialog_view';
const _columnsKey = 'file_dialog_columns';
const _navigationShareKey = 'file_dialog_navigation_share';
const _sortKey = 'file_dialog_sort';
const _hiddenKey = 'file_dialog_show_hidden';
const _lastFolderKey = 'file_dialog_last_folder';

/// The requests the dialog makes, replaceable in tests.
class FileDialogBackend {
  const FileDialogBackend({
    this.listPlaces = listFileDialogPlaces,
    this.resolve = resolveFileDialogPath,
    this.listFolder = listFileDialogFolder,
    this.createFolder = createFileDialogFolder,
    this.rename = renameFileDialogEntry,
  });

  final Future<ListFileDialogPlacesResponse> Function() listPlaces;
  final Future<ResolveFileDialogPathResponse> Function(
    String typed,
    FileDialogLocation current,
  )
  resolve;
  final Future<List<FileDialogEntry>> Function(FileDialogLocation location)
  listFolder;
  final Future<FileDialogLocation> Function(
    FileDialogLocation parent,
    String name,
  )
  createFolder;
  final Future<FileDialogLocation> Function(
    FileDialogLocation location,
    String name,
  )
  rename;
}

/// Something the dialog must tell the person, as File Explorer's message
/// boxes do.
sealed class FileDialogNotice {
  const FileDialogNotice();
}

/// A request failed; the dialog explains it.
class FileDialogFailureNotice extends FileDialogNotice {
  const FileDialogFailureNotice(this.error, this.subject);

  final Object error;

  /// What was asked for: a typed path, a folder or a new name.
  final String subject;
}

/// A typed file name names no file (open modes).
class FileDialogFileNotFoundNotice extends FileDialogNotice {
  const FileDialogFileNotFoundNotice(this.typed);

  final String typed;
}

/// A typed path goes through a folder that does not exist.
class FileDialogPathNotFoundNotice extends FileDialogNotice {
  const FileDialogPathNotFoundNotice(this.typed);

  final String typed;
}

/// When picking folders: the typed name is not a folder there.
class FileDialogFolderNotFoundNotice extends FileDialogNotice {
  const FileDialogFolderNotFoundNotice(this.typed);

  final String typed;
}

/// The state of one open file dialog: where it is, what it lists, what is
/// selected and how it is shown.
class FileDialogController extends ChangeNotifier {
  FileDialogController({
    required this.options,
    required this.onAccept,
    required this.onNotice,
    required this.confirmReplace,
    this.backend = const FileDialogBackend(),
  }) : fileTypeIndex = options.fileTypes.isEmpty
           ? 0
           : options.initialFileTypeIndex.clamp(
               0,
               options.fileTypes.length - 1,
             ) {
    fileName.text = options.suggestedName ?? '';
  }

  final FileDialogOptions options;
  final FileDialogBackend backend;
  final void Function(FileDialogResult result) onAccept;

  /// Shows a notice; the future completes when the person dismisses it.
  final Future<void> Function(FileDialogNotice notice) onNotice;

  /// Asks whether to replace an existing file; true to replace.
  final Future<bool> Function(String name) confirmReplace;

  final fileName = TextEditingController();
  final search = TextEditingController();

  ListFileDialogPlacesResponse? places;
  FileDialogLocation? current;
  List<FileDialogEntry>? _entries;
  Object? listingError;
  bool loading = false;
  int _loadToken = 0;

  /// Places to go back to, the most recent last.
  final _back = <FileDialogLocation>[];
  final _forward = <FileDialogLocation>[];

  FileDialogView view = FileDialogView.details;
  FileDialogSort sort = const FileDialogSort(FileDialogColumn.name, true);
  bool showHidden = false;

  /// Widths of the details columns that were resized; the others keep
  /// their default, and the name fills the room left until it is resized.
  final columnWidths = <FileDialogColumn, double>{};

  /// Details columns switched off from the header's menu. The name is
  /// always shown.
  final hiddenColumns = <FileDialogColumn>{};

  /// The navigation pane's share of the dialog's width, as last dragged.
  double navigationShare = defaultNavigationShare;

  static const defaultNavigationShare = 0.22;
  int fileTypeIndex;

  /// Selected items, and the one a range selection starts from.
  final selection = <FileDialogLocation>{};
  FileDialogLocation? anchor;

  /// The item keyboard focus is on.
  FileDialogLocation? focused;

  /// The item being renamed in place.
  FileDialogLocation? renaming;

  /// When picking folders: the folder the name box names, and the name it
  /// shows for it. That is the selected folder, or the folder being shown
  /// when none is selected, and picking with the name unchanged picks it.
  (FileDialogLocation, String)? _namedFolder;

  bool _disposed = false;

  bool get canGoBack => _back.isNotEmpty;
  bool get canGoForward => _forward.isNotEmpty;
  bool get canGoUp => current != null && parentOf(current!) != null;
  List<FileDialogLocation> get backHistory => List.unmodifiable(_back);
  List<FileDialogLocation> get forwardHistory => List.unmodifiable(_forward);

  bool get multiple => options.mode == FileDialogMode.openFiles;
  bool get picksFolders => options.mode == FileDialogMode.pickFolder;
  bool get saves => options.mode == FileDialogMode.saveFile;

  FileDialogFileType? get fileType =>
      options.fileTypes.isEmpty ? null : options.fileTypes[fileTypeIndex];

  /// A filter typed into the file name box, such as `*.flac`, which
  /// replaces the file type's until the type changes.
  String? typedFilter;

  /// The items shown: folders always, files of the selected type (none
  /// when picking folders), hidden ones only when asked, and those matching
  /// the search.
  List<FileDialogEntry> get visibleEntries {
    final entries = _entries;
    if (entries == null) return const [];
    final query = search.text.trim().toLowerCase();
    final filter = typedFilter;
    return sortEntries(
      entries.where((entry) {
        if (entry.hidden && !showHidden) return false;
        if (query.isNotEmpty && !entry.name.toLowerCase().contains(query)) {
          return false;
        }
        if (isFolder(entry)) return true;
        if (picksFolders) return false;
        if (filter != null) return _matchesWildcard(entry.name, filter);
        return fileType?.matches(entry.name) ?? true;
      }),
      sort,
    );
  }

  bool get hasEntries => _entries != null;

  Future<void> start() async {
    await _restorePreferences();
    unawaited(_loadPlaces());
    final initial = options.initialFolder ?? await _lastFolder();
    if (initial != null && await navigate(initial, remember: false)) return;
    // The last folder may be gone: start at home instead.
    final home = await _home();
    if (home != null && home != initial) {
      current = null;
      await navigate(home, remember: false);
    }
  }

  Future<FileDialogLocation?> _home() async {
    try {
      final places = this.places ?? await backend.listPlaces();
      for (final place in [...places.folders, ...places.pinned]) {
        if (place.name case FileDialogPlaceNameKnown(
          folder: FileDialogKnownFolder.home,
        )) {
          return place.location;
        }
      }
      if (places.folders.isNotEmpty) return places.folders.first.location;
      if (places.volumes.isNotEmpty) return places.volumes.first.location;
    } catch (_) {
      // Without places the dialog shows its failure in the item area.
    }
    return null;
  }

  Future<void> _loadPlaces() async {
    try {
      places = await backend.listPlaces();
    } catch (_) {
      places = const ListFileDialogPlacesResponse(
        pinned: [],
        folders: [],
        volumes: [],
      );
    }
    _notify();
  }

  Future<void> _restorePreferences() async {
    final settings = SettingsManager();
    view =
        FileDialogView.fromStorageName(
          await settings.getValue<String>(_viewKey),
        ) ??
        FileDialogView.details;
    final columns = await settings.getValue<Map<dynamic, dynamic>>(_columnsKey);
    if (columns != null) {
      FileDialogColumn? column(Object? name) => FileDialogColumn.values
          .where((value) => value.name == name)
          .firstOrNull;
      final widths = columns['widths'];
      if (widths is Map) {
        for (final MapEntry(:key, :value) in widths.entries) {
          final known = column(key);
          if (known != null && value is num) {
            columnWidths[known] = value.toDouble();
          }
        }
      }
      final hidden = columns['hidden'];
      if (hidden is List) {
        hiddenColumns.addAll(
          hidden
              .map(column)
              .whereType<FileDialogColumn>()
              .where((value) => value != FileDialogColumn.name),
        );
      }
    }
    final share = await settings.getValue<num>(_navigationShareKey);
    if (share != null) navigationShare = share.toDouble();
    final sortValue = await settings.getValue<String>(_sortKey);
    if (sortValue != null) {
      final [column, order] = '$sortValue:'.split(':').take(2).toList();
      sort = FileDialogSort(
        FileDialogColumn.values.firstWhere(
          (value) => value.name == column,
          orElse: () => FileDialogColumn.name,
        ),
        order != 'descending',
      );
    }
    showHidden = await settings.getValue<bool>(_hiddenKey) ?? false;
  }

  Future<FileDialogLocation?> _lastFolder() async {
    final saved = await SettingsManager().getValue<Map<dynamic, dynamic>>(
      _lastFolderKey,
    );
    final path = saved?['path'];
    if (path is! String) return null;
    final uri = saved?['tree'];
    return FileDialogLocation(
      root: uri is String
          ? FileDialogRootDocumentTree(uri: uri)
          : const FileDialogRootFileSystem(),
      path: path,
    );
  }

  Future<void> _rememberFolder(FileDialogLocation location) =>
      SettingsManager().setValue(_lastFolderKey, {
        'path': location.path,
        if (location.root case FileDialogRootDocumentTree(:final uri))
          'tree': uri,
      });

  /// Show the folder at [location]. [remember] puts the folder left behind
  /// on the back history. A folder that cannot be read is explained and the
  /// dialog stays where it was, as File Explorer does; only when there is no
  /// folder to stay in is the failure shown in the item area.
  Future<bool> navigate(
    FileDialogLocation location, {
    bool remember = true,
    FileDialogLocation? select,
  }) async {
    final previous = current;
    final shown = await _show(location, select: select);
    if (shown && remember && previous != null && previous != location) {
      _back.add(previous);
      _forward.clear();
      _notify();
    }
    return shown;
  }

  Future<bool> _show(
    FileDialogLocation location, {
    FileDialogLocation? select,
  }) async {
    final token = ++_loadToken;
    loading = true;
    renaming = null;
    _notify();
    List<FileDialogEntry> entries;
    try {
      entries = await backend.listFolder(location);
    } catch (error) {
      if (token != _loadToken || _disposed) return false;
      loading = false;
      if (current == null || current == location) {
        current = location;
        _entries = null;
        listingError = error;
      } else {
        unawaited(onNotice(FileDialogFailureNotice(error, location.path)));
      }
      _notify();
      return false;
    }
    if (token != _loadToken || _disposed) return false;
    final changed = current != location;
    current = location;
    _entries = entries;
    listingError = null;
    loading = false;
    if (changed) {
      selection.clear();
      anchor = null;
      focused = null;
      search.clear();
      typedFilter = null;
    } else {
      selection.removeWhere(
        (selected) => !entries.any((entry) => entry.location == selected),
      );
    }
    if (select != null && entries.any((entry) => entry.location == select)) {
      selection
        ..clear()
        ..add(select);
      anchor = select;
      focused = select;
    }
    if (changed && picksFolders) _syncFileName();
    unawaited(_rememberFolder(location));
    _notify();
    return true;
  }

  Future<void> back() async {
    if (_back.isEmpty) return;
    final target = _back.last;
    final left = current;
    if (!await _show(target, select: left)) return;
    _back.removeLast();
    if (left != null) _forward.add(left);
    _notify();
  }

  Future<void> forward() async {
    if (_forward.isEmpty) return;
    final target = _forward.last;
    final left = current;
    if (!await _show(target)) return;
    _forward.removeLast();
    if (left != null) _back.add(left);
    _notify();
  }

  /// Jump to the [index]th place of the back history (0 is the oldest), as
  /// the recent locations menu does.
  Future<void> travelBack(int index) async {
    if (index < 0 || index >= _back.length) return;
    final target = _back[index];
    final left = current;
    if (!await _show(target)) return;
    final skipped = _back.sublist(index + 1);
    _back.removeRange(index, _back.length);
    if (left != null) _forward.add(left);
    _forward.addAll(skipped.reversed);
    _notify();
  }

  Future<void> up() async {
    final location = current;
    final parent = location == null ? null : parentOf(location);
    if (parent != null) await navigate(parent, select: location);
  }

  Future<void> refresh() async {
    final location = current;
    if (location != null) await _show(location);
  }

  void setView(FileDialogView next) {
    if (next == view) return;
    view = next;
    _saveLater(_viewKey, () => view.storageName);
    _notify();
  }

  /// Settings written once a burst of changes (a zoom, a column drag) has
  /// settled, or when the dialog closes.
  final _pendingSaves = <String, (Timer, Object Function())>{};

  void _saveLater(String key, Object Function() value) {
    _pendingSaves[key]?.$1.cancel();
    _pendingSaves[key] = (
      Timer(const Duration(milliseconds: 300), () => _saveNow(key)),
      value,
    );
  }

  void _saveNow(String key) {
    final pending = _pendingSaves.remove(key);
    if (pending == null) return;
    pending.$1.cancel();
    unawaited(SettingsManager().setValue(key, pending.$2()));
  }

  /// Resize a details column; null gives it back its default width, and
  /// the name back the room the others leave.
  void setColumnWidth(FileDialogColumn column, double? width) {
    if (width == null) {
      columnWidths.remove(column);
    } else {
      columnWidths[column] = width;
    }
    _saveColumns();
    _notify();
  }

  /// Resize several details columns at once, as Size All Columns to Fit
  /// does.
  void setColumnWidths(Map<FileDialogColumn, double> widths) {
    columnWidths.addAll(widths);
    _saveColumns();
    _notify();
  }

  void setColumnShown(FileDialogColumn column, bool shown) {
    if (column == FileDialogColumn.name) return;
    if (shown) {
      hiddenColumns.remove(column);
    } else {
      hiddenColumns.add(column);
    }
    _saveColumns();
    _notify();
  }

  void _saveColumns() {
    _saveLater(
      _columnsKey,
      () => {
        'widths': {
          for (final MapEntry(:key, :value) in columnWidths.entries)
            key.name: value,
        },
        'hidden': [for (final column in hiddenColumns) column.name],
      },
    );
  }

  /// Set the navigation pane's share of the width while it is dragged;
  /// [save] keeps it for the next dialog once the drag ends.
  void setNavigationShare(double share, {bool save = false}) {
    navigationShare = share;
    if (save) {
      unawaited(SettingsManager().setValue(_navigationShareKey, share));
    }
    _notify();
  }

  void setSort(FileDialogSort next) {
    sort = next;
    unawaited(
      SettingsManager().setValue(
        _sortKey,
        '${next.column.name}:${next.ascending ? 'ascending' : 'descending'}',
      ),
    );
    _notify();
  }

  void setShowHidden(bool value) {
    showHidden = value;
    unawaited(SettingsManager().setValue(_hiddenKey, value));
    _notify();
  }

  void setFileType(int index) {
    fileTypeIndex = index;
    typedFilter = null;
    if (saves) _applyExtension();
    _notify();
  }

  void searchChanged() => _notify();

  /// In the save dialog, the name keeps the extension of the chosen type.
  void _applyExtension() {
    final extension = fileType?.extensions.firstOrNull;
    final name = fileName.text.trim();
    if (extension == null || name.isEmpty) return;
    final stem = stemOf(name);
    final current = extensionOf(name);
    final known = options.fileTypes.any(
      (type) => type.extensions.contains(current),
    );
    if (current.isEmpty || known) fileName.text = '$stem.$extension';
  }

  // Selection.

  /// Select [location] as a click does: alone, toggled with Ctrl, as a
  /// range from the anchor with Shift.
  void select(
    FileDialogLocation location, {
    bool toggle = false,
    bool range = false,
  }) {
    final visible = visibleEntries;
    if (range && multiple && anchor != null) {
      final from = visible.indexWhere((entry) => entry.location == anchor);
      final to = visible.indexWhere((entry) => entry.location == location);
      if (from >= 0 && to >= 0) {
        if (!toggle) selection.clear();
        final (low, high) = from <= to ? (from, to) : (to, from);
        selection.addAll(
          visible.sublist(low, high + 1).map((entry) => entry.location),
        );
      }
    } else if (toggle && multiple) {
      if (!selection.remove(location)) selection.add(location);
      anchor = location;
    } else {
      selection
        ..clear()
        ..add(location);
      anchor = location;
    }
    focused = location;
    _syncFileName();
    _notify();
  }

  void selectAll() {
    if (!multiple) return;
    selection
      ..clear()
      ..addAll(
        visibleEntries
            .where((entry) => !isFolder(entry))
            .map((e) => e.location),
      );
    _syncFileName();
    _notify();
  }

  void clearSelection() {
    selection.clear();
    if (picksFolders) _syncFileName();
    _notify();
  }

  /// Move the keyboard focus [delta] items and select there; with [range],
  /// extend the selection from the anchor.
  void moveFocus(int delta, {bool range = false, bool keepSelection = false}) {
    final visible = visibleEntries;
    if (visible.isEmpty) return;
    final index = visible.indexWhere((entry) => entry.location == focused);
    final next = index < 0
        ? (delta >= 0 ? 0 : visible.length - 1)
        : (index + delta).clamp(0, visible.length - 1);
    final target = visible[next].location;
    if (keepSelection) {
      focused = target;
      _notify();
      return;
    }
    select(target, range: range);
  }

  void focusEdge({required bool end, bool range = false}) {
    final visible = visibleEntries;
    if (visible.isEmpty) return;
    select((end ? visible.last : visible.first).location, range: range);
  }

  /// Type-ahead: select the next item whose name starts with [prefix].
  void selectByPrefix(String prefix) {
    final visible = visibleEntries;
    if (visible.isEmpty || prefix.isEmpty) return;
    final lower = prefix.toLowerCase();
    final start = visible.indexWhere((entry) => entry.location == focused);
    // A repeated single letter cycles through the items it starts.
    final from = prefix.length == 1
        ? start + 1
        : start.clamp(0, visible.length);
    for (var offset = 0; offset < visible.length; offset++) {
      final entry = visible[(from + offset) % visible.length];
      if (entry.name.toLowerCase().startsWith(lower)) {
        select(entry.location);
        return;
      }
    }
  }

  List<FileDialogEntry> get selectedEntries => visibleEntries
      .where((entry) => selection.contains(entry.location))
      .toList();

  /// The file name box follows the selection, as in Windows: the file's
  /// name, several quoted names, or a folder's name when picking folders.
  void _syncFileName() {
    final selected = selectedEntries;
    final files = selected.where((entry) => !isFolder(entry)).toList();
    if (picksFolders) {
      final folders = selected.where(isFolder).toList();
      final shown = current;
      final named = folders.length == 1
          ? (folders.first.location, folders.first.name)
          : shown == null
          ? null
          : (shown, nameOf(shown));
      if (named != null) {
        fileName.text = named.$2;
        _namedFolder = named;
      }
      return;
    }
    if (files.isEmpty) return;
    fileName.text = files.length == 1
        ? files.first.name
        : files.map((entry) => '"${entry.name}"').join(' ');
  }

  // Opening and accepting.

  /// Open an item as a double click does: enter a folder, pick a file.
  Future<void> activate(FileDialogEntry entry) async {
    if (isFolder(entry)) {
      await navigate(entry.location);
      return;
    }
    if (picksFolders || entry.kind != FileDialogEntryKind.file) return;
    if (saves) {
      fileName.text = entry.name;
      await submit();
      return;
    }
    _accept([entry.location]);
  }

  /// The confirm button and Enter in the file name box.
  Future<void> submit() async {
    final location = current;
    if (location == null) return;
    final typed = fileName.text.trim();

    if (picksFolders) {
      // The name the dialog put in the box stands for its folder: after
      // opening a folder the box names it, and its name is not looked up
      // inside it.
      final named = _namedFolder;
      if (named != null && typed == named.$2) {
        _accept([named.$1]);
        return;
      }
      if (typed.isEmpty) {
        final folders = selectedEntries.where(isFolder).toList();
        _accept([folders.isEmpty ? location : folders.first.location]);
        return;
      }
      await _resolveAndAct(typed, location);
      return;
    }

    if (typed.isEmpty) {
      final folders = selectedEntries.where(isFolder).toList();
      if (folders.length == 1) await navigate(folders.first.location);
      return;
    }

    if (_isWildcard(typed)) {
      typedFilter = typed;
      fileName.clear();
      _notify();
      return;
    }

    final names = _quotedNames(typed);
    if (names.length > 1 && multiple) {
      final picked = <FileDialogLocation>[];
      for (final name in names) {
        final resolved = await _resolve(name, location);
        if (resolved == null) return;
        if (resolved.target != FileDialogPathTarget.file) {
          unawaited(onNotice(FileDialogFileNotFoundNotice(name)));
          return;
        }
        picked.add(resolved.location);
      }
      _accept(picked);
      return;
    }

    await _resolveAndAct(names.isEmpty ? typed : names.first, location);
  }

  Future<ResolveFileDialogPathResponse?> _resolve(
    String typed,
    FileDialogLocation location,
  ) async {
    try {
      return await backend.resolve(typed, location);
    } catch (error) {
      unawaited(onNotice(FileDialogFailureNotice(error, typed)));
      return null;
    }
  }

  Future<void> _resolveAndAct(String typed, FileDialogLocation location) async {
    var name = typed;
    if (saves && extensionOf(name).isEmpty) {
      final extension = fileType?.extensions.firstOrNull;
      if (extension != null) name = '$name.$extension';
    }
    final resolved = await _resolve(name, location);
    if (resolved == null || _disposed) return;
    switch (resolved.target) {
      case FileDialogPathTarget.folder:
        if (picksFolders) {
          _accept([resolved.location]);
        } else {
          fileName.clear();
          await navigate(resolved.location);
        }
      case FileDialogPathTarget.file:
        if (picksFolders) {
          unawaited(onNotice(FileDialogFolderNotFoundNotice(typed)));
        } else if (saves) {
          if (await confirmReplace(nameOf(resolved.location))) {
            _accept([resolved.location]);
          }
        } else {
          _accept([resolved.location]);
        }
      case FileDialogPathTarget.missing:
        if (saves) {
          _accept([resolved.location]);
        } else if (picksFolders) {
          unawaited(onNotice(FileDialogFolderNotFoundNotice(typed)));
        } else {
          unawaited(onNotice(FileDialogFileNotFoundNotice(typed)));
        }
      case FileDialogPathTarget.unreachable:
        unawaited(onNotice(FileDialogPathNotFoundNotice(typed)));
    }
  }

  /// Address bar entry: a folder to show, or for the open modes a file to
  /// pick, as File Explorer's address bar treats it.
  Future<void> goTo(String typed) async {
    final location = current;
    if (location == null || typed.trim().isEmpty) return;
    final resolved = await _resolve(typed, location);
    if (resolved == null || _disposed) return;
    switch (resolved.target) {
      case FileDialogPathTarget.folder:
        await navigate(resolved.location);
      case FileDialogPathTarget.file when !picksFolders && !saves:
        _accept([resolved.location]);
      case FileDialogPathTarget.file:
        final parent = parentOf(resolved.location);
        if (parent != null) {
          await navigate(parent, select: resolved.location);
          _syncFileName();
        }
      case FileDialogPathTarget.missing || FileDialogPathTarget.unreachable:
        unawaited(
          onNotice(FileDialogFailureNotice(const _NotFound(), typed.trim())),
        );
    }
  }

  void _accept(List<FileDialogLocation> locations) {
    if (locations.isEmpty) return;
    onAccept(FileDialogResult(locations, fileTypeIndex));
  }

  // Changes to the folder.

  /// Create a folder named after [baseName] (made unique as Windows does)
  /// and start renaming it.
  Future<void> createFolder(String baseName) async {
    final location = current;
    final entries = _entries;
    if (location == null || entries == null) return;
    final taken = entries.map((entry) => entry.name.toLowerCase()).toSet();
    var name = baseName;
    for (var index = 2; taken.contains(name.toLowerCase()); index++) {
      name = '$baseName ($index)';
    }
    try {
      final created = await backend.createFolder(location, name);
      await _show(location, select: created);
      renaming = created;
      _notify();
    } catch (error) {
      unawaited(onNotice(FileDialogFailureNotice(error, name)));
    }
  }

  void startRename() {
    final target = focused ?? selection.firstOrNull;
    if (target == null || selection.length > 1) return;
    renaming = target;
    _notify();
  }

  void cancelRename() {
    renaming = null;
    _notify();
  }

  /// Rename the item being renamed. A failure keeps the editor open so the
  /// name can be corrected.
  Future<bool> commitRename(String name) async {
    final target = renaming;
    if (target == null) return true;
    final trimmed = name.trim();
    if (trimmed.isEmpty || trimmed == nameOf(target)) {
      cancelRename();
      return true;
    }
    try {
      final renamed = await backend.rename(target, trimmed);
      renaming = null;
      final location = current;
      if (location != null) await _show(location, select: renamed);
      return true;
    } catch (error) {
      // The editor stays open; it takes the focus back once the person has
      // read why the name was refused.
      await onNotice(FileDialogFailureNotice(error, trimmed));
      return false;
    }
  }

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    _pendingSaves.keys.toList().forEach(_saveNow);
    fileName.dispose();
    search.dispose();
    super.dispose();
  }
}

/// A typed address that names nothing; shown like a missing folder.
class _NotFound implements Exception {
  const _NotFound();
}

bool isNotFound(Object error) =>
    error is _NotFound ||
    fileDialogFailureOf(error) == FileDialogFailure.notFound;

bool _isWildcard(String typed) =>
    !typed.startsWith('"') && (typed.contains('*') || typed.contains('?'));

/// Whether [name] matches a `*.flac;*.mp3` style filter.
bool _matchesWildcard(String name, String filter) {
  final lower = name.toLowerCase();
  return filter
      .split(';')
      .map((part) => part.trim())
      .where((part) {
        return part.isNotEmpty;
      })
      .any((pattern) {
        final expression = RegExp(
          '^${RegExp.escape(pattern.toLowerCase()).replaceAll(r'\*', '.*').replaceAll(r'\?', '.')}\$',
        );
        return expression.hasMatch(lower);
      });
}

/// The names in `"a.flac" "b.flac"`, or the text itself when unquoted.
List<String> _quotedNames(String typed) {
  if (!typed.startsWith('"')) return [typed];
  return RegExp(r'"([^"]+)"')
      .allMatches(typed)
      .map((match) => match.group(1)!.trim())
      .where((name) => name.isNotEmpty)
      .toList();
}
