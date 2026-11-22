import 'dart:async';
import 'dart:math' as math;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:intl/intl.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../../bindings/bindings.dart';
import '../../../../l10n/app_localizations.dart';
import '../../../api/file_dialog.dart';
import '../../../api/request_failure.dart';
import '../../../l10n.dart';

import '../file_dialog_controller.dart';
import '../file_dialog_model.dart';
import '../file_dialog_names.dart';
import '../file_dialog_thumbnails.dart';
import 'item_icon.dart';

/// Opens a context menu for an item, or for the folder when [entry] is
/// null, at a position in global coordinates.
typedef FileDialogContextMenuCallback =
    void Function(FileDialogEntry? entry, Offset globalPosition);

const double _rowHeight = 28;
const double _listCellWidth = 240;
const double _tileWidth = 260;
const double _tileHeight = 64;
const double _contentRowHeight = 60;
const Duration _doubleClick = Duration(milliseconds: 500);
const Duration _typeAheadPause = Duration(seconds: 1);

/// The item area of the file dialog in all eight File Explorer layouts,
/// with mouse, touch and keyboard selection, type-ahead and renaming in
/// place.
class FileDialogItemsView extends StatefulWidget {
  const FileDialogItemsView({
    super.key,
    required this.controller,
    required this.names,
    required this.thumbnails,
    required this.focusNode,
    required this.onContextMenu,
  });

  final FileDialogController controller;
  final FileDialogNames names;
  final FileDialogThumbnails thumbnails;
  final FocusNode focusNode;
  final FileDialogContextMenuCallback onContextMenu;

  @override
  State<FileDialogItemsView> createState() => FileDialogItemsViewState();
}

class FileDialogItemsViewState extends State<FileDialogItemsView> {
  final _vertical = ScrollController();
  final _horizontal = ScrollController();
  final _itemKeys = <FileDialogLocation, GlobalKey>{};

  /// Items per row in grids, rows per column in the list layout.
  int _perLine = 1;
  double _viewport = 0;

  DateTime _lastClick = DateTime.fromMillisecondsSinceEpoch(0);
  FileDialogLocation? _lastClicked;

  String _typed = '';
  Timer? _typedTimer;

  /// Set by an item's pointer handler, which runs before the background's,
  /// so a click on an item does not also count as a click on empty space.
  bool _itemHit = false;

  double _dateWidth = 160;
  double _typeWidth = 130;
  double _sizeWidth = 96;

  FileDialogController get _controller => widget.controller;

  @override
  void initState() {
    super.initState();
    widget.focusNode.addListener(_focusChanged);
  }

  @override
  void didUpdateWidget(covariant FileDialogItemsView oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.focusNode != widget.focusNode) {
      oldWidget.focusNode.removeListener(_focusChanged);
      widget.focusNode.addListener(_focusChanged);
    }
  }

  /// The keyboard focus outline follows the focus in and out of the items.
  void _focusChanged() {
    if (mounted) setState(() {});
  }

  @override
  void dispose() {
    widget.focusNode.removeListener(_focusChanged);
    _vertical.dispose();
    _horizontal.dispose();
    _typedTimer?.cancel();
    super.dispose();
  }

  GlobalKey _keyFor(FileDialogLocation location) =>
      _itemKeys.putIfAbsent(location, GlobalKey.new);

  /// Where the context menu key opens the menu: under the focused item.
  Offset? focusedItemPosition() {
    final focused = _controller.focused ?? _controller.selection.firstOrNull;
    final box = focused == null
        ? null
        : _itemKeys[focused]?.currentContext?.findRenderObject() as RenderBox?;
    if (box == null || !box.attached) return null;
    return box.localToGlobal(Offset(24, box.size.height));
  }

  // Keyboard.

  bool get _columnMajor => _controller.viewMode == FileDialogViewMode.list;

  bool get _isGrid {
    final mode = _controller.viewMode;
    return mode.isGrid ||
        mode == FileDialogViewMode.smallIcons ||
        mode == FileDialogViewMode.tiles;
  }

  KeyEventResult _onKey(FocusNode node, KeyEvent event) {
    if (event is! KeyDownEvent && event is! KeyRepeatEvent) {
      return KeyEventResult.ignored;
    }
    if (_controller.renaming != null) return KeyEventResult.ignored;
    final keys = HardwareKeyboard.instance;
    // Alt combinations (back, forward, up, the address bar) are the
    // dialog's own shortcuts.
    if (keys.isAltPressed) return KeyEventResult.ignored;
    final shift = keys.isShiftPressed;
    final control = keys.isControlPressed || keys.isMetaPressed;
    final key = event.logicalKey;

    void move(int delta) {
      _controller.moveFocus(delta, range: shift, keepSelection: control);
      _revealFocused();
    }

    final across = _isGrid ? 1 : (_columnMajor ? _perLine : 0);
    final down = _isGrid ? _perLine : 1;
    switch (key) {
      case LogicalKeyboardKey.arrowDown:
        move(down);
      case LogicalKeyboardKey.arrowUp:
        move(-down);
      case LogicalKeyboardKey.arrowRight when across > 0:
        move(across);
      case LogicalKeyboardKey.arrowLeft when across > 0:
        move(-across);
      case LogicalKeyboardKey.pageDown:
        move(math.max(1, _linesOnScreen()) * down);
      case LogicalKeyboardKey.pageUp:
        move(-math.max(1, _linesOnScreen()) * down);
      case LogicalKeyboardKey.home:
        _controller.focusEdge(end: false, range: shift);
        _revealFocused();
      case LogicalKeyboardKey.end:
        _controller.focusEdge(end: true, range: shift);
        _revealFocused();
      case LogicalKeyboardKey.space when control:
        final focused = _controller.focused;
        if (focused != null) _controller.select(focused, toggle: true);
      case LogicalKeyboardKey.enter || LogicalKeyboardKey.numpadEnter:
        _openSelection();
      case LogicalKeyboardKey.backspace:
        unawaited(_controller.back());
      case LogicalKeyboardKey.f2:
        _controller.startRename();
      case LogicalKeyboardKey.contextMenu:
        _openMenuFromKeyboard();
      case LogicalKeyboardKey.f10 when shift:
        _openMenuFromKeyboard();
      case LogicalKeyboardKey.keyA when control:
        _controller.selectAll();
      default:
        final character = event.character;
        if (character == null ||
            control ||
            character.trim().isEmpty ||
            character.codeUnitAt(0) < 0x20) {
          return KeyEventResult.ignored;
        }
        _typeAhead(character);
    }
    return KeyEventResult.handled;
  }

  void _openMenuFromKeyboard() {
    final focused = _controller.focused;
    final entry = _controller.visibleEntries
        .where((entry) => entry.location == focused)
        .firstOrNull;
    final position = focusedItemPosition() ?? _centerOfView();
    widget.onContextMenu(entry, position);
  }

  Offset _centerOfView() {
    final box = context.findRenderObject() as RenderBox?;
    return box == null
        ? Offset.zero
        : box.localToGlobal(box.size.center(Offset.zero));
  }

  void _openSelection() {
    final selected = _controller.selectedEntries;
    if (selected.length == 1) {
      unawaited(_controller.activate(selected.first));
    } else if (selected.isNotEmpty) {
      unawaited(_controller.submit());
    }
  }

  void _typeAhead(String character) {
    _typedTimer?.cancel();
    _typed += character;
    _typedTimer = Timer(_typeAheadPause, () => _typed = '');
    _controller.selectByPrefix(_typed);
    _revealFocused();
  }

  int _linesOnScreen() {
    final extent = _lineExtent();
    return extent <= 0 ? 1 : (_viewport / extent).floor();
  }

  double _lineExtent() => switch (_controller.viewMode) {
    FileDialogViewMode.details => _rowHeight,
    FileDialogViewMode.content => _contentRowHeight,
    FileDialogViewMode.tiles => _tileHeight,
    FileDialogViewMode.smallIcons || FileDialogViewMode.list => 24,
    final mode => _gridCellHeight(mode),
  };

  /// Scroll so the focused item is in view.
  void _revealFocused() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      final focused = _controller.focused;
      final index = _controller.visibleEntries.indexWhere(
        (entry) => entry.location == focused,
      );
      if (index < 0) return;
      final extent = _lineExtent();
      final scroll = _columnMajor ? _horizontal : _vertical;
      if (!scroll.hasClients) return;
      final line = _columnMajor || _isGrid
          ? index ~/ math.max(1, _perLine)
          : index;
      final lineExtent = _columnMajor ? _listCellWidth : extent;
      final start = line * lineExtent;
      final end = start + lineExtent;
      final position = scroll.position;
      if (start < position.pixels) {
        scroll.jumpTo(start);
      } else if (end > position.pixels + position.viewportDimension) {
        scroll.jumpTo(
          math.min(position.maxScrollExtent, end - position.viewportDimension),
        );
      }
    });
  }

  // Pointer.

  void _pointerDown(PointerDownEvent event, FileDialogEntry entry) {
    _itemHit = true;
    widget.focusNode.requestFocus();
    if (event.buttons == kSecondaryButton) {
      if (!_controller.selection.contains(entry.location)) {
        _controller.select(entry.location);
      }
      widget.onContextMenu(entry, event.position);
      return;
    }
    if (event.buttons != kPrimaryButton) return;
    if (_controller.renaming == entry.location) return;
    // Touch opens folders with one tap, as touch file pickers do.
    if (event.kind == PointerDeviceKind.touch) {
      _controller.select(entry.location);
      if (isFolder(entry)) unawaited(_controller.activate(entry));
      return;
    }
    final now = DateTime.now();
    final again =
        _lastClicked == entry.location &&
        now.difference(_lastClick) < _doubleClick;
    if (again) {
      _lastClicked = null;
      unawaited(_controller.activate(entry));
      return;
    }
    final keys = HardwareKeyboard.instance;
    _controller.select(
      entry.location,
      toggle: keys.isControlPressed || keys.isMetaPressed,
      range: keys.isShiftPressed,
    );
    _lastClick = now;
    _lastClicked = entry.location;
  }

  void _backgroundPointerDown(PointerDownEvent event) {
    if (_itemHit) {
      _itemHit = false;
      return;
    }
    widget.focusNode.requestFocus();
    if (event.buttons == kSecondaryButton) {
      widget.onContextMenu(null, event.position);
    } else if (event.buttons == kPrimaryButton &&
        !HardwareKeyboard.instance.isControlPressed) {
      _controller.clearSelection();
    }
  }

  // Building.

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    final controller = _controller;
    final theme = FluentTheme.of(context);
    final entries = controller.visibleEntries;

    final Widget body;
    if (controller.listingError != null && !controller.hasEntries) {
      body = _Message(
        icon: Symbols.folder_off,
        text: describeFileDialogError(
          s,
          controller.listingError!,
          widget.names.location(controller.current!),
        ),
      );
    } else if (!controller.hasEntries) {
      body = controller.loading
          ? const Center(child: ProgressRing())
          : const SizedBox.expand();
    } else if (entries.isEmpty) {
      body = _Message(
        text: controller.search.text.trim().isEmpty
            ? s.fileDialogEmptyFolder
            : s.fileDialogNoMatches,
      );
    } else {
      body = LayoutBuilder(
        builder: (context, constraints) =>
            _layout(context, constraints, entries),
      );
    }

    return Semantics(
      label: s.fileDialogItems,
      value: controller.hasEntries
          ? s.fileDialogItemCount(entries.length)
          : null,
      container: true,
      child: Focus(
        focusNode: widget.focusNode,
        onKeyEvent: _onKey,
        child: Listener(
          behavior: HitTestBehavior.opaque,
          onPointerDown: _backgroundPointerDown,
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onLongPressStart: (details) =>
                widget.onContextMenu(null, details.globalPosition),
            child: DefaultTextStyle.merge(
              style: theme.typography.body,
              child: body,
            ),
          ),
        ),
      ),
    );
  }

  Widget _layout(
    BuildContext context,
    BoxConstraints constraints,
    List<FileDialogEntry> entries,
  ) {
    final mode = _controller.viewMode;
    _viewport = mode == FileDialogViewMode.list
        ? constraints.maxWidth
        : constraints.maxHeight;
    switch (mode) {
      case FileDialogViewMode.details:
        _perLine = 1;
        return _details(context, constraints, entries);
      case FileDialogViewMode.content:
        _perLine = 1;
        return Scrollbar(
          controller: _vertical,
          child: ListView.builder(
            controller: _vertical,
            itemExtent: _contentRowHeight,
            itemCount: entries.length,
            itemBuilder: (context, index) => _item(
              entries[index],
              (selected) => _ContentRow(
                entry: entries[index],
                names: widget.names,
                thumbnails: widget.thumbnails,
                date: _date(context, entries[index]),
                renaming: _renameField(entries[index]),
              ),
              row: true,
            ),
          ),
        );
      case FileDialogViewMode.list:
        final rows = math.max(1, (constraints.maxHeight / 24).floor());
        _perLine = rows;
        return Scrollbar(
          controller: _horizontal,
          child: GridView.builder(
            controller: _horizontal,
            scrollDirection: Axis.horizontal,
            gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
              crossAxisCount: rows,
              mainAxisExtent: _listCellWidth,
            ),
            itemCount: entries.length,
            itemBuilder: (context, index) => _item(
              entries[index],
              (selected) => _SmallRow(
                entry: entries[index],
                names: widget.names,
                renaming: _renameField(entries[index]),
              ),
              row: true,
            ),
          ),
        );
      case FileDialogViewMode.smallIcons:
        final columns = math.max(
          1,
          (constraints.maxWidth / _listCellWidth).floor(),
        );
        _perLine = columns;
        return _grid(
          columns,
          24,
          entries,
          (entry) => _SmallRow(
            entry: entry,
            names: widget.names,
            renaming: _renameField(entry),
          ),
          row: true,
        );
      case FileDialogViewMode.tiles:
        final columns = math.max(
          1,
          (constraints.maxWidth / _tileWidth).floor(),
        );
        _perLine = columns;
        return _grid(
          columns,
          _tileHeight,
          entries,
          (entry) => _Tile(
            entry: entry,
            names: widget.names,
            thumbnails: widget.thumbnails,
            renaming: _renameField(entry),
          ),
        );
      case FileDialogViewMode.mediumIcons ||
          FileDialogViewMode.largeIcons ||
          FileDialogViewMode.extraLargeIcons:
        final width = _gridCellWidth(mode);
        final columns = math.max(1, (constraints.maxWidth / width).floor());
        _perLine = columns;
        return _grid(
          columns,
          _gridCellHeight(mode),
          entries,
          (entry) => _IconCell(
            entry: entry,
            names: widget.names,
            thumbnails: widget.thumbnails,
            size: mode.iconSize,
            renaming: _renameField(entry),
          ),
        );
    }
  }

  static double _gridCellWidth(FileDialogViewMode mode) => mode.iconSize + 40;

  static double _gridCellHeight(FileDialogViewMode mode) => mode.iconSize + 56;

  Widget _grid(
    int columns,
    double height,
    List<FileDialogEntry> entries,
    Widget Function(FileDialogEntry entry) cell, {
    bool row = false,
  }) {
    return Scrollbar(
      controller: _vertical,
      child: GridView.builder(
        controller: _vertical,
        padding: const EdgeInsets.all(4),
        gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
          crossAxisCount: columns,
          mainAxisExtent: height,
          mainAxisSpacing: 2,
          crossAxisSpacing: 2,
        ),
        itemCount: entries.length,
        itemBuilder: (context, index) =>
            _item(entries[index], (selected) => cell(entries[index]), row: row),
      ),
    );
  }

  Widget _details(
    BuildContext context,
    BoxConstraints constraints,
    List<FileDialogEntry> entries,
  ) {
    final columns = _DetailsColumns(_dateWidth, _typeWidth, _sizeWidth);
    return Column(
      children: [
        _DetailsHeader(
          columns: columns,
          sort: _controller.sort,
          onSort: (column) =>
              _controller.setSort(_controller.sort.toggled(column)),
          onResize: (column, delta) => setState(() {
            switch (column) {
              case FileDialogSortColumn.dateModified:
                _dateWidth = (_dateWidth - delta).clamp(60, 400);
              case FileDialogSortColumn.type:
                _typeWidth = (_typeWidth - delta).clamp(60, 400);
              case FileDialogSortColumn.size:
                _sizeWidth = (_sizeWidth - delta).clamp(60, 400);
              case FileDialogSortColumn.name:
                break;
            }
          }),
        ),
        Expanded(
          child: LayoutBuilder(
            builder: (context, inner) {
              _viewport = inner.maxHeight;
              return Scrollbar(
                controller: _vertical,
                child: ListView.builder(
                  controller: _vertical,
                  itemExtent: _rowHeight,
                  itemCount: entries.length,
                  itemBuilder: (context, index) {
                    final entry = entries[index];
                    return _item(
                      entry,
                      (selected) => _DetailsRow(
                        entry: entry,
                        names: widget.names,
                        columns: columns,
                        date: _date(context, entry),
                        renaming: _renameField(entry),
                      ),
                      row: true,
                    );
                  },
                ),
              );
            },
          ),
        ),
      ],
    );
  }

  String _date(BuildContext context, FileDialogEntry entry) {
    final seconds = entry.modified?.toInt();
    if (seconds == null) return '';
    final time = DateTime.fromMillisecondsSinceEpoch(seconds * 1000);
    final locale = Intl.verifiedLocale(
      Localizations.localeOf(context).languageCode,
      DateFormat.localeExists,
      onFailure: (_) => 'en',
    );
    return DateFormat.yMd(locale).add_Hm().format(time);
  }

  Widget? _renameField(FileDialogEntry entry) {
    if (_controller.renaming != entry.location) return null;
    return _RenameField(
      key: ValueKey(entry.location),
      name: entry.name,
      selectStemOnly: !isFolder(entry),
      onCommit: (name) async {
        final done = await _controller.commitRename(name);
        if (done) widget.focusNode.requestFocus();
        return done;
      },
      onCancel: () {
        _controller.cancelRename();
        widget.focusNode.requestFocus();
      },
    );
  }

  /// One item with its selection, hover, focus and pointer behaviour.
  Widget _item(
    FileDialogEntry entry,
    Widget Function(bool selected) child, {
    required bool row,
  }) {
    final selected = _controller.selection.contains(entry.location);
    final focused =
        widget.focusNode.hasFocus && _controller.focused == entry.location;
    return _ItemFrame(
      key: _keyFor(entry.location),
      label: _semanticLabel(entry),
      selected: selected,
      focused: focused,
      row: row,
      onPointerDown: (event) => _pointerDown(event, entry),
      onLongPress: (position) {
        _controller.select(entry.location);
        widget.onContextMenu(entry, position);
      },
      child: child(selected),
    );
  }

  String _semanticLabel(FileDialogEntry entry) {
    final parts = [entry.name, widget.names.entryType(entry)];
    if (!isFolder(entry)) parts.add(shortSize(entry.size.toInt()));
    return parts.join(', ');
  }
}

/// Explains why a folder or item could not be read or changed.
String describeFileDialogError(
  AppLocalizations s,
  Object error,
  String subject,
) {
  if (isNotFound(error)) return s.fileDialogNotFound(subject);
  return switch (fileDialogFailureOf(error)) {
    FileDialogFailure.notFound => s.fileDialogNotFound(subject),
    FileDialogFailure.notAFolder => s.fileDialogNotAFolder(subject),
    FileDialogFailure.permissionDenied => s.fileDialogPermissionDenied(subject),
    FileDialogFailure.unavailable => s.fileDialogUnavailable(subject),
    FileDialogFailure.alreadyExists => s.fileDialogAlreadyExists(subject),
    FileDialogFailure.invalidName => s.fileDialogInvalidName(subject),
    null => requestErrorText(s, error),
  };
}

class _Message extends StatelessWidget {
  const _Message({required this.text, this.icon});

  final String text;
  final IconData? icon;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (icon != null) ...[
              Icon(
                icon,
                size: 32,
                color: theme.resources.textFillColorSecondary,
              ),
              const SizedBox(height: 12),
            ],
            Text(
              text,
              textAlign: TextAlign.center,
              style: TextStyle(color: theme.resources.textFillColorSecondary),
            ),
          ],
        ),
      ),
    );
  }
}

/// Selection, hover and keyboard focus around one item, painted as Fluent's
/// list tiles paint them: a fill, and for rows an accent bar at the start.
class _ItemFrame extends StatefulWidget {
  const _ItemFrame({
    super.key,
    required this.label,
    required this.selected,
    required this.focused,
    required this.row,
    required this.onPointerDown,
    required this.onLongPress,
    required this.child,
  });

  final String label;
  final bool selected;
  final bool focused;
  final bool row;
  final void Function(PointerDownEvent event) onPointerDown;
  final void Function(Offset globalPosition) onLongPress;
  final Widget child;

  @override
  State<_ItemFrame> createState() => _ItemFrameState();
}

class _ItemFrameState extends State<_ItemFrame> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final accent = theme.accentColor.defaultBrushFor(theme.brightness);
    final fill = ButtonThemeData.uncheckedInputColor(theme, {
      if (_hovered || widget.selected) WidgetState.hovered,
    }, transparentWhenNone: true);
    return Semantics(
      label: widget.label,
      selected: widget.selected,
      button: true,
      excludeSemantics: true,
      child: MouseRegion(
        onEnter: (_) => setState(() => _hovered = true),
        onExit: (_) => setState(() => _hovered = false),
        child: Listener(
          behavior: HitTestBehavior.opaque,
          onPointerDown: (event) {
            widget.onPointerDown(event);
          },
          child: GestureDetector(
            onLongPressStart: (details) =>
                widget.onLongPress(details.globalPosition),
            child: FocusBorder(
              focused: widget.focused,
              renderOutside: false,
              child: Container(
                decoration: BoxDecoration(
                  color: fill,
                  borderRadius: BorderRadius.circular(4),
                  border: !widget.row && widget.selected
                      ? Border.all(color: accent.withValues(alpha: 0.6))
                      : null,
                ),
                child: widget.row
                    ? Row(
                        children: [
                          Container(
                            width: 3,
                            height: 14,
                            margin: const EdgeInsetsDirectional.only(
                              start: 2,
                              end: 5,
                            ),
                            decoration: BoxDecoration(
                              color: widget.selected
                                  ? accent
                                  : Colors.transparent,
                              borderRadius: BorderRadius.circular(100),
                            ),
                          ),
                          Expanded(child: widget.child),
                        ],
                      )
                    : widget.child,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _DetailsColumns {
  const _DetailsColumns(this.date, this.type, this.size);

  final double date;
  final double type;
  final double size;
}

class _DetailsHeader extends StatelessWidget {
  const _DetailsHeader({
    required this.columns,
    required this.sort,
    required this.onSort,
    required this.onResize,
  });

  final _DetailsColumns columns;
  final FileDialogSort sort;
  final void Function(FileDialogSortColumn column) onSort;
  final void Function(FileDialogSortColumn column, double delta) onResize;

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    final theme = FluentTheme.of(context);
    Widget header(FileDialogSortColumn column, String label, {double? width}) {
      final active = sort.column == column;
      final cell = HoverButton(
        onPressed: () => onSort(column),
        semanticLabel: label,
        builder: (context, states) => Container(
          height: _rowHeight,
          color: ButtonThemeData.uncheckedInputColor(
            theme,
            states,
            transparentWhenNone: true,
          ),
          padding: const EdgeInsets.symmetric(horizontal: 8),
          child: Row(
            mainAxisAlignment: column == FileDialogSortColumn.size
                ? MainAxisAlignment.end
                : MainAxisAlignment.start,
            children: [
              Flexible(
                child: Text(
                  label,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    color: theme.resources.textFillColorSecondary,
                  ),
                ),
              ),
              if (active) ...[
                const SizedBox(width: 4),
                Icon(
                  sort.ascending
                      ? Symbols.keyboard_arrow_up
                      : Symbols.keyboard_arrow_down,
                  size: 14,
                  color: theme.resources.textFillColorSecondary,
                ),
              ],
            ],
          ),
        ),
      );
      final resizable = width == null
          ? Expanded(child: cell)
          : SizedBox(
              width: width,
              child: Stack(
                children: [
                  Positioned.fill(child: cell),
                  // Dragging a column's start edge resizes it, as in Windows.
                  Positioned(
                    left: 0,
                    top: 4,
                    bottom: 4,
                    width: 6,
                    child: MouseRegion(
                      cursor: SystemMouseCursors.resizeColumn,
                      child: GestureDetector(
                        behavior: HitTestBehavior.opaque,
                        onHorizontalDragUpdate: (details) =>
                            onResize(column, details.delta.dx),
                        child: Center(
                          child: Container(
                            width: 1,
                            color: theme.resources.dividerStrokeColorDefault,
                          ),
                        ),
                      ),
                    ),
                  ),
                ],
              ),
            );
      return resizable;
    }

    return Container(
      height: _rowHeight,
      // Lines up with the rows past their selection bar.
      padding: const EdgeInsetsDirectional.only(start: 10),
      child: Row(
        children: [
          header(FileDialogSortColumn.name, s.fileDialogName),
          header(
            FileDialogSortColumn.dateModified,
            s.fileDialogDateModified,
            width: columns.date,
          ),
          header(FileDialogSortColumn.type, s.type, width: columns.type),
          header(
            FileDialogSortColumn.size,
            s.fileDialogSize,
            width: columns.size,
          ),
        ],
      ),
    );
  }
}

class _DetailsRow extends StatelessWidget {
  const _DetailsRow({
    required this.entry,
    required this.names,
    required this.columns,
    required this.date,
    required this.renaming,
  });

  final FileDialogEntry entry;
  final FileDialogNames names;
  final _DetailsColumns columns;
  final String date;
  final Widget? renaming;

  @override
  Widget build(BuildContext context) {
    final secondary = TextStyle(
      color: FluentTheme.of(context).resources.textFillColorSecondary,
    );
    Widget cell(String text, double width, {bool end = false}) => SizedBox(
      width: width,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8),
        child: Text(
          text,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          textAlign: end ? TextAlign.end : TextAlign.start,
          style: secondary,
        ),
      ),
    );
    return Opacity(
      opacity: entry.hidden ? 0.6 : 1,
      child: Row(
        children: [
          FileDialogItemIcon(
            entry: entry,
            names: names,
            size: 16,
            thumbnails: null,
          ),
          const SizedBox(width: 8),
          Expanded(
            child:
                renaming ??
                Text(entry.name, maxLines: 1, overflow: TextOverflow.ellipsis),
          ),
          cell(date, columns.date),
          cell(names.entryType(entry), columns.type),
          cell(
            isFolder(entry) ? '' : detailsSize(entry.size.toInt()),
            columns.size,
            end: true,
          ),
        ],
      ),
    );
  }
}

class _SmallRow extends StatelessWidget {
  const _SmallRow({
    required this.entry,
    required this.names,
    required this.renaming,
  });

  final FileDialogEntry entry;
  final FileDialogNames names;
  final Widget? renaming;

  @override
  Widget build(BuildContext context) {
    return Opacity(
      opacity: entry.hidden ? 0.6 : 1,
      child: Row(
        children: [
          FileDialogItemIcon(
            entry: entry,
            names: names,
            size: 16,
            thumbnails: null,
          ),
          const SizedBox(width: 8),
          Expanded(
            child:
                renaming ??
                Text(entry.name, maxLines: 1, overflow: TextOverflow.ellipsis),
          ),
        ],
      ),
    );
  }
}

class _IconCell extends StatelessWidget {
  const _IconCell({
    required this.entry,
    required this.names,
    required this.thumbnails,
    required this.size,
    required this.renaming,
  });

  final FileDialogEntry entry;
  final FileDialogNames names;
  final FileDialogThumbnails thumbnails;
  final double size;
  final Widget? renaming;

  @override
  Widget build(BuildContext context) {
    return Opacity(
      opacity: entry.hidden ? 0.6 : 1,
      child: Padding(
        padding: const EdgeInsets.all(6),
        child: Column(
          children: [
            FileDialogItemIcon(
              entry: entry,
              names: names,
              size: size,
              thumbnails: thumbnails,
            ),
            const SizedBox(height: 4),
            Expanded(
              child:
                  renaming ??
                  Text(
                    entry.name,
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    textAlign: TextAlign.center,
                  ),
            ),
          ],
        ),
      ),
    );
  }
}

class _Tile extends StatelessWidget {
  const _Tile({
    required this.entry,
    required this.names,
    required this.thumbnails,
    required this.renaming,
  });

  final FileDialogEntry entry;
  final FileDialogNames names;
  final FileDialogThumbnails thumbnails;
  final Widget? renaming;

  @override
  Widget build(BuildContext context) {
    final secondary = TextStyle(
      color: FluentTheme.of(context).resources.textFillColorSecondary,
    );
    return Opacity(
      opacity: entry.hidden ? 0.6 : 1,
      child: Padding(
        padding: const EdgeInsets.all(6),
        child: Row(
          children: [
            FileDialogItemIcon(
              entry: entry,
              names: names,
              size: 48,
              thumbnails: thumbnails,
            ),
            const SizedBox(width: 8),
            Expanded(
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  renaming ??
                      Text(
                        entry.name,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                      ),
                  Text(
                    names.entryType(entry),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: secondary,
                  ),
                  if (!isFolder(entry))
                    Text(shortSize(entry.size.toInt()), style: secondary),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _ContentRow extends StatelessWidget {
  const _ContentRow({
    required this.entry,
    required this.names,
    required this.thumbnails,
    required this.date,
    required this.renaming,
  });

  final FileDialogEntry entry;
  final FileDialogNames names;
  final FileDialogThumbnails thumbnails;
  final String date;
  final Widget? renaming;

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    final theme = FluentTheme.of(context);
    final secondary = TextStyle(color: theme.resources.textFillColorSecondary);
    return Opacity(
      opacity: entry.hidden ? 0.6 : 1,
      child: Container(
        decoration: BoxDecoration(
          border: Border(
            bottom: BorderSide(
              color: theme.resources.dividerStrokeColorDefault,
            ),
          ),
        ),
        padding: const EdgeInsets.symmetric(vertical: 6),
        child: Row(
          children: [
            FileDialogItemIcon(
              entry: entry,
              names: names,
              size: 32,
              thumbnails: thumbnails,
            ),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  renaming ??
                      Text(
                        entry.name,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                      ),
                  Text(names.entryType(entry), style: secondary, maxLines: 1),
                ],
              ),
            ),
            SizedBox(
              width: 220,
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  if (date.isNotEmpty)
                    Text(
                      '${s.fileDialogDateModified}: $date',
                      style: secondary,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
                  if (!isFolder(entry))
                    Text(
                      '${s.fileDialogSize}: ${shortSize(entry.size.toInt())}',
                      style: secondary,
                      maxLines: 1,
                    ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// The name of an item being renamed: the name without its extension is
/// selected, Enter or leaving the box keeps the new name, Escape drops it.
class _RenameField extends StatefulWidget {
  const _RenameField({
    super.key,
    required this.name,
    required this.selectStemOnly,
    required this.onCommit,
    required this.onCancel,
  });

  final String name;
  final bool selectStemOnly;
  final Future<bool> Function(String name) onCommit;
  final VoidCallback onCancel;

  @override
  State<_RenameField> createState() => _RenameFieldState();
}

class _RenameFieldState extends State<_RenameField> {
  late final _text = TextEditingController(text: widget.name);
  final _focus = FocusNode();
  bool _committing = false;
  bool _cancelled = false;

  @override
  void initState() {
    super.initState();
    final end = widget.selectStemOnly
        ? stemOf(widget.name).length
        : widget.name.length;
    _text.selection = TextSelection(baseOffset: 0, extentOffset: end);
    // Leaving the box keeps the name, except while a refused name is
    // being explained, which _commit is still waiting on.
    _focus.addListener(() {
      if (!_focus.hasFocus && !_cancelled) _commit();
    });
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  @override
  void dispose() {
    _text.dispose();
    _focus.dispose();
    super.dispose();
  }

  Future<void> _commit() async {
    if (_committing) return;
    _committing = true;
    final done = await widget.onCommit(_text.text);
    _committing = false;
    // The explanation has been closed: back to correcting the name.
    if (!done && mounted) _focus.requestFocus();
  }

  @override
  Widget build(BuildContext context) {
    return CallbackShortcuts(
      bindings: {
        const SingleActivator(LogicalKeyboardKey.escape): () {
          _cancelled = true;
          widget.onCancel();
        },
      },
      child: TextBox(
        controller: _text,
        focusNode: _focus,
        onSubmitted: (_) => _commit(),
        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 2),
        autocorrect: false,
        enableSuggestions: false,
      ),
    );
  }
}
