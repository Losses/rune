import 'dart:async';
import 'dart:math' as math;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:intl/intl.dart' show DateFormat, Intl;
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
import 'drag_handle.dart';
import 'item_icon.dart';

/// Opens a context menu for an item, or for the folder when [entry] is
/// null, at a position in global coordinates.
typedef FileDialogContextMenuCallback =
    void Function(FileDialogEntry? entry, Offset globalPosition);

/// Opens the details header's menu, for [column] when the pointer was on
/// one, at a position in global coordinates.
typedef FileDialogColumnMenuCallback =
    void Function(FileDialogColumn? column, Offset globalPosition);

const double _rowHeight = 28;

/// Rows a finger can hit: what File Explorer gives touch.
const double _touchRowHeight = 40;
const double _listCellWidth = 240;
const double _tileWidth = 260;
const double _tileHeight = 64;
const double _contentRowHeight = 60;
const Duration _doubleClick = Duration(milliseconds: 500);
const Duration _typeAheadPause = Duration(seconds: 1);

/// The room before a row's content: the selection bar and its margins.
const double _rowLead = 10;

/// A cell's padding on each side in the details layout.
const double _cellPadding = 8;

/// The name column's icon and the gap after it.
const double _nameIcon = 16 + 8;

/// The narrowest a name may get before a narrow view drops other columns.
const double _narrowestName = 160;
const double _narrowestColumn = 40;

/// The room an editable text keeps after its text for the caret: the
/// rename box's 1 px caret and Flutter's 1 px gap.
const double _caretRoom = 2;

/// The scroll one notch of a mouse wheel makes: Flutter reports about 53
/// logical pixels a notch on Linux and 60 on Windows.
const double _wheelNotch = 50;

/// The item area of the file dialog in all of File Explorer's layouts, with
/// mouse, touch and keyboard selection, type-ahead, renaming in place and
/// zooming with Ctrl and the mouse wheel.
class FileDialogItemsView extends StatefulWidget {
  const FileDialogItemsView({
    super.key,
    required this.controller,
    required this.names,
    required this.thumbnails,
    required this.focusNode,
    required this.onContextMenu,
    required this.onColumnMenu,
  });

  final FileDialogController controller;
  final FileDialogNames names;
  final FileDialogThumbnails thumbnails;
  final FocusNode focusNode;
  final FileDialogContextMenuCallback onContextMenu;
  final FileDialogColumnMenuCallback onColumnMenu;

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

  /// The item area's width, for fitting the details columns to it.
  double _width = 0;

  DateTime _lastClick = DateTime.fromMillisecondsSinceEpoch(0);
  FileDialogLocation? _lastClicked;

  /// When anything in the items was last pressed, for telling a click that
  /// renames from one that is part of a double click.
  DateTime _lastPress = DateTime.fromMillisecondsSinceEpoch(0);

  /// The item whose name the pointer went down on; set by the name, whose
  /// handler runs before the item's.
  FileDialogLocation? _nameHit;

  /// Renaming that starts once no second click follows.
  Timer? _renameTimer;

  String _typed = '';
  Timer? _typedTimer;

  /// The width of the column whose divider is being dragged, when the drag
  /// started.
  double _columnAtDragStart = 0;

  /// Wheel notches turned with Ctrl held that did not change the view yet.
  double _zoomNotches = 0;
  Timer? _zoomIdle;

  /// Set by an item's pointer handler, which runs before the background's,
  /// so a click on an item does not also count as a click on empty space.
  bool _itemHit = false;

  /// The kind of the last pointer pressed in the items. After a touch the
  /// rows grow to finger size.
  PointerDeviceKind _pointer = PointerDeviceKind.mouse;

  static bool _isTouch(PointerDeviceKind kind) =>
      kind == PointerDeviceKind.touch || kind == PointerDeviceKind.stylus;

  bool get _touch => _isTouch(_pointer);

  double get _rowExtent => _touch ? _touchRowHeight : _rowHeight;

  double get _smallRowExtent => _touch ? _touchRowHeight : 24;

  void _notePointer(PointerDeviceKind kind) {
    if (kind != _pointer && mounted) setState(() => _pointer = kind);
  }

  FileDialogController get _controller => widget.controller;

  @override
  void initState() {
    super.initState();
    widget.focusNode.addListener(_focusChanged);
    FocusManager.instance.addHighlightModeListener(_highlightChanged);
  }

  /// The keyboard focus outline shows only while the keyboard is in use.
  void _highlightChanged(FocusHighlightMode mode) {
    if (mounted) setState(() {});
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
    FocusManager.instance.removeHighlightModeListener(_highlightChanged);
    _vertical.dispose();
    _horizontal.dispose();
    _typedTimer?.cancel();
    _renameTimer?.cancel();
    _zoomIdle?.cancel();
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

  FileDialogLayout get _layout => _controller.view.layout;

  bool get _columnMajor => _layout == FileDialogLayout.list;

  bool get _isGrid =>
      _layout == FileDialogLayout.icons ||
      _layout == FileDialogLayout.smallIcons ||
      _layout == FileDialogLayout.tiles;

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

  double _lineExtent() => switch (_layout) {
    FileDialogLayout.details => _rowExtent,
    FileDialogLayout.content => _contentRowHeight,
    FileDialogLayout.tiles => _tileHeight,
    FileDialogLayout.smallIcons || FileDialogLayout.list => _smallRowExtent,
    FileDialogLayout.icons => _gridCellHeight(_controller.view.iconSize),
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
    final onName = _nameHit == entry.location;
    _nameHit = null;
    _notePointer(event.kind);
    // A finger selects when it lifts (see [_tap]), so a scroll that starts
    // on an item neither selects nor opens it.
    if (_isTouch(event.kind)) return;
    final now = DateTime.now();
    final quiet = now.difference(_lastPress) >= _doubleClick;
    _lastPress = now;
    _renameTimer?.cancel();
    final hadFocus = widget.focusNode.hasFocus;
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
    final again =
        _lastClicked == entry.location &&
        now.difference(_lastClick) < _doubleClick;
    if (again) {
      _lastClicked = null;
      unawaited(_controller.activate(entry));
      return;
    }
    final keys = HardwareKeyboard.instance;
    final modified =
        keys.isControlPressed || keys.isMetaPressed || keys.isShiftPressed;
    final aloneBefore =
        _controller.selection.length == 1 &&
        _controller.selection.contains(entry.location);
    _controller.select(
      entry.location,
      toggle: keys.isControlPressed || keys.isMetaPressed,
      range: keys.isShiftPressed,
    );
    _lastClick = now;
    _lastClicked = entry.location;
    // File Explorer: a click on the name of the item selected alone, with
    // no other click a double-click time before or after it, renames it.
    if (onName && aloneBefore && hadFocus && quiet && !modified) {
      _renameTimer = Timer(_doubleClick, () {
        if (!mounted) return;
        final selection = _controller.selection;
        if (selection.length == 1 && selection.contains(entry.location)) {
          _controller.startRename();
        }
      });
    }
  }

  /// A tap of a finger or pen: a folder opens at once, as in touch file
  /// pickers; a file is selected, and where several can be picked each tap
  /// adds or removes one, since touch has no Ctrl key.
  void _tap(FileDialogEntry entry, TapUpDetails details) {
    if (!_isTouch(details.kind)) return;
    if (_controller.renaming == entry.location) return;
    widget.focusNode.requestFocus();
    if (isFolder(entry)) {
      _controller.select(entry.location);
      unawaited(_controller.activate(entry));
      return;
    }
    _controller.select(entry.location, toggle: _controller.multiple);
  }

  void _backgroundPointerDown(PointerDownEvent event) {
    if (_itemHit) {
      _itemHit = false;
      return;
    }
    _nameHit = null;
    _notePointer(event.kind);
    if (_isTouch(event.kind)) return;
    _lastPress = DateTime.now();
    _renameTimer?.cancel();
    widget.focusNode.requestFocus();
    if (event.buttons == kSecondaryButton) {
      widget.onContextMenu(null, event.position);
    } else if (event.buttons == kPrimaryButton &&
        !HardwareKeyboard.instance.isControlPressed) {
      _controller.clearSelection();
    }
  }

  /// A finger tapping empty space clears the selection when it lifts.
  void _backgroundTap(TapUpDetails details) {
    if (_isTouch(details.kind)) {
      _controller.clearSelection();
    }
  }

  /// Ctrl and the mouse wheel zoom, as in File Explorer: icons change size
  /// smoothly, and below Medium icons each notch is the next layout.
  void _pointerSignal(PointerSignalEvent event) {
    if (event is! PointerScrollEvent) return;
    final keys = HardwareKeyboard.instance;
    if (!keys.isControlPressed && !keys.isMetaPressed) return;
    // The views' scroll physics refuse the scroll while Ctrl is held, so
    // this is the only handler for it.
    GestureBinding.instance.pointerSignalResolver.register(event, (event) {
      zoom((event as PointerScrollEvent).scrollDelta.dy / _wheelNotch);
    });
  }

  /// Zoom by [notches] of the mouse wheel: down (positive) for smaller
  /// items.
  void zoom(double notches) {
    _zoomIdle?.cancel();
    _zoomIdle = Timer(
      const Duration(milliseconds: 400),
      () => _zoomNotches = 0,
    );
    _zoomNotches = (_zoomNotches + notches).clamp(-2.0, 2.0);
    final view = _controller.view;
    final next = view.zoomed(_zoomNotches);
    if (next != view) {
      _zoomNotches = 0;
      _controller.setView(next);
    }
  }

  // Details columns.

  /// The details columns shown, in order, with their widths in a view
  /// [width] wide. The name fills the room the others leave until it is
  /// resized; a view too narrow for that drops columns, the type first,
  /// then the size, then the date, keeping the name readable.
  List<(FileDialogColumn, double)> _detailsColumns(double width) {
    final controller = _controller;
    double widthOf(FileDialogColumn column) =>
        controller.columnWidths[column] ?? column.defaultWidth;
    final others = [
      for (final column in FileDialogColumn.values)
        if (column != FileDialogColumn.name &&
            !controller.hiddenColumns.contains(column))
          column,
    ];
    final name = controller.columnWidths[FileDialogColumn.name];
    if (name != null) {
      return [
        (FileDialogColumn.name, name),
        for (final column in others) (column, widthOf(column)),
      ];
    }
    double room() =>
        width -
        _rowLead -
        others.fold(0.0, (total, column) => total + widthOf(column));
    for (final drop in const [
      FileDialogColumn.type,
      FileDialogColumn.size,
      FileDialogColumn.dateModified,
    ]) {
      if (room() >= _narrowestName) break;
      others.remove(drop);
    }
    return [
      (FileDialogColumn.name, math.max(room(), _narrowestName)),
      for (final column in others) (column, widthOf(column)),
    ];
  }

  String _cellText(FileDialogColumn column, FileDialogEntry entry) =>
      switch (column) {
        FileDialogColumn.name => entry.name,
        FileDialogColumn.dateModified => _date(context, entry),
        FileDialogColumn.type => widget.names.entryType(entry),
        FileDialogColumn.size =>
          isFolder(entry) ? '' : detailsSize(entry.size.toInt()),
      };

  /// The width that shows [column]'s header and every value in it whole.
  double _fittedWidth(FileDialogColumn column) {
    final s = S.of(context);
    // The style the rows inherit (see [build]).
    final style = DefaultTextStyle.of(
      context,
    ).style.merge(FluentTheme.of(context).typography.body);
    final direction = Directionality.of(context);
    final scaler = MediaQuery.textScalerOf(context);
    double measure(String text) {
      if (text.isEmpty) return 0;
      final painter = TextPainter(
        text: TextSpan(text: text, style: style),
        maxLines: 1,
        textDirection: direction,
        textScaler: scaler,
      )..layout();
      final width = painter.width;
      painter.dispose();
      return width;
    }

    // The header leaves room for the sort arrow.
    var widest =
        measure(fileDialogColumnLabel(s, column)) + 18 + _cellPadding * 2;
    // The name's cell starts with its icon and pads only its end.
    final around = column == FileDialogColumn.name
        ? _nameIcon + _cellPadding
        : _cellPadding * 2;
    for (final entry in _controller.visibleEntries) {
      widest = math.max(widest, around + measure(_cellText(column, entry)));
    }
    return widest.ceilToDouble();
  }

  /// Size All Columns to Fit, as File Explorer's header menu offers.
  void sizeAllColumnsToFit() {
    _controller.setColumnWidths({
      for (final (column, _) in _detailsColumns(_width))
        column: _fittedWidth(column),
    });
  }

  /// Size Column to Fit, as File Explorer's header menu offers.
  void sizeColumnToFit(FileDialogColumn column) {
    _controller.setColumnWidth(column, _fittedWidth(column));
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
            _layoutItems(context, constraints, entries),
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
          onPointerSignal: _pointerSignal,
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTapUp: _backgroundTap,
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

  Widget _layoutItems(
    BuildContext context,
    BoxConstraints constraints,
    List<FileDialogEntry> entries,
  ) {
    final view = _controller.view;
    _width = constraints.maxWidth;
    _viewport = view.layout == FileDialogLayout.list
        ? constraints.maxWidth
        : constraints.maxHeight;
    switch (view.layout) {
      case FileDialogLayout.details:
        _perLine = 1;
        return _details(context, constraints, entries);
      case FileDialogLayout.content:
        _perLine = 1;
        return Scrollbar(
          controller: _vertical,
          child: ListView.builder(
            controller: _vertical,
            physics: const _ZoomablePhysics(),
            itemExtent: _contentRowHeight,
            itemCount: entries.length,
            itemBuilder: (context, index) => _item(
              entries[index],
              _ContentRow(
                entry: entries[index],
                names: widget.names,
                thumbnails: widget.thumbnails,
                date: _date(context, entries[index]),
                name: _name(entries[index]),
                wide: constraints.maxWidth >= 480,
              ),
              row: true,
            ),
          ),
        );
      case FileDialogLayout.list:
        final rows = math.max(
          1,
          (constraints.maxHeight / _smallRowExtent).floor(),
        );
        _perLine = rows;
        return Scrollbar(
          controller: _horizontal,
          child: GridView.builder(
            controller: _horizontal,
            physics: const _ZoomablePhysics(),
            scrollDirection: Axis.horizontal,
            gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
              crossAxisCount: rows,
              mainAxisExtent: _listCellWidth,
            ),
            itemCount: entries.length,
            itemBuilder: (context, index) => _item(
              entries[index],
              _SmallRow(
                entry: entries[index],
                names: widget.names,
                name: _name(entries[index]),
              ),
              row: true,
            ),
          ),
        );
      case FileDialogLayout.smallIcons:
        final columns = math.max(
          1,
          (constraints.maxWidth / _listCellWidth).floor(),
        );
        _perLine = columns;
        return _grid(
          columns,
          _smallRowExtent,
          entries,
          (entry) =>
              _SmallRow(entry: entry, names: widget.names, name: _name(entry)),
          row: true,
        );
      case FileDialogLayout.tiles:
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
            name: _name(entry),
          ),
        );
      case FileDialogLayout.icons:
        final size = view.iconSize;
        final columns = math.max(
          1,
          (constraints.maxWidth / _gridCellWidth(size)).floor(),
        );
        _perLine = columns;
        return _grid(
          columns,
          _gridCellHeight(size),
          entries,
          (entry) => _IconCell(
            entry: entry,
            names: widget.names,
            thumbnails: widget.thumbnails,
            size: size,
            name: _name(entry, centered: true),
          ),
        );
    }
  }

  static double _gridCellWidth(double iconSize) => iconSize + 40;

  static double _gridCellHeight(double iconSize) => iconSize + 56;

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
        physics: const _ZoomablePhysics(),
        padding: const EdgeInsets.all(4),
        gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
          crossAxisCount: columns,
          mainAxisExtent: height,
          mainAxisSpacing: 2,
          crossAxisSpacing: 2,
        ),
        itemCount: entries.length,
        itemBuilder: (context, index) =>
            _item(entries[index], cell(entries[index]), row: row),
      ),
    );
  }

  Widget _details(
    BuildContext context,
    BoxConstraints constraints,
    List<FileDialogEntry> entries,
  ) {
    final columns = _detailsColumns(constraints.maxWidth);
    final total = columns.fold(_rowLead, (sum, column) => sum + column.$2);
    final content = SizedBox(
      width: math.max(total, constraints.maxWidth),
      height: constraints.maxHeight,
      child: Column(
        children: [
          _DetailsHeader(
            onPointerDown: (_) {
              _itemHit = true;
              _renameTimer?.cancel();
            },
            columns: columns,
            sort: _controller.sort,
            label: (column) => fileDialogColumnLabel(S.of(context), column),
            onSort: (column) =>
                _controller.setSort(_controller.sort.toggled(column)),
            onResizeStart: (column, width) => _columnAtDragStart = width,
            onResize: (column, distance) => _controller.setColumnWidth(
              column,
              math.max(
                column == FileDialogColumn.name
                    ? _nameIcon + _cellPadding * 2
                    : _narrowestColumn,
                _columnAtDragStart + distance,
              ),
            ),
            onFit: sizeColumnToFit,
            onMenu: widget.onColumnMenu,
          ),
          Expanded(
            child: LayoutBuilder(
              builder: (context, inner) {
                _viewport = inner.maxHeight;
                return ListView.builder(
                  controller: _vertical,
                  physics: const _ZoomablePhysics(),
                  itemExtent: _rowExtent,
                  itemCount: entries.length,
                  itemBuilder: (context, index) {
                    final entry = entries[index];
                    return _item(
                      entry,
                      _DetailsRow(
                        entry: entry,
                        names: widget.names,
                        columns: columns,
                        name: _name(entry),
                        cell: (column) => _cellText(column, entry),
                      ),
                      row: true,
                    );
                  },
                );
              },
            ),
          ),
        ],
      ),
    );
    // The header and rows scroll sideways together when the columns are
    // wider than the view; the vertical scroll bar stays at the view's edge.
    return Scrollbar(
      controller: _horizontal,
      child: Scrollbar(
        controller: _vertical,
        notificationPredicate: (notification) => notification.depth == 1,
        child: SingleChildScrollView(
          controller: _horizontal,
          physics: const _ZoomablePhysics(),
          scrollDirection: Axis.horizontal,
          child: content,
        ),
      ),
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

  /// An item's name, or the box renaming it in its place with its text
  /// where the name was. A click on the name itself starts renaming (see
  /// [_pointerDown]); the icon and the space beside the name do not.
  Widget _name(FileDialogEntry entry, {bool centered = false}) {
    final align = centered ? TextAlign.center : TextAlign.start;
    final Widget name;
    if (_controller.renaming == entry.location) {
      final field = _RenameField(
        key: ValueKey(entry.location),
        name: entry.name,
        selectStemOnly: !isFolder(entry),
        textAlign: align,
        singleLine: !centered,
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
      // The box keeps room for the caret after the text. A centred name
      // gets that room on top of the label's width, and as much before the
      // text, so its lines are centred where the label's were.
      name = centered
          ? LayoutBuilder(
              builder: (context, constraints) => OverflowBox(
                alignment: Alignment.topCenter,
                minWidth: 0,
                maxWidth: constraints.maxWidth + _caretRoom * 2,
                child: Padding(
                  padding: const EdgeInsetsDirectional.only(start: _caretRoom),
                  child: field,
                ),
              ),
            )
          : field;
    } else {
      name = Listener(
        onPointerDown: (_) => _nameHit = entry.location,
        child: Text(
          entry.name,
          maxLines: centered ? 2 : 1,
          overflow: TextOverflow.ellipsis,
          textAlign: align,
        ),
      );
    }
    return Align(
      alignment: centered
          ? Alignment.topCenter
          : AlignmentDirectional.centerStart,
      widthFactor: centered ? null : 1,
      heightFactor: centered ? null : 1,
      child: name,
    );
  }

  /// One item with its selection, hover, focus and pointer behaviour.
  Widget _item(FileDialogEntry entry, Widget child, {required bool row}) {
    final selected = _controller.selection.contains(entry.location);
    final focused =
        widget.focusNode.hasFocus &&
        FocusManager.instance.highlightMode == FocusHighlightMode.traditional &&
        _controller.focused == entry.location;
    return _ItemFrame(
      key: _keyFor(entry.location),
      label: _semanticLabel(entry),
      selected: selected,
      focused: focused,
      row: row,
      onPointerDown: (event) => _pointerDown(event, entry),
      onTap: (details) => _tap(entry, details),
      onLongPress: (position) {
        _controller.select(entry.location);
        widget.onContextMenu(entry, position);
      },
      child: child,
    );
  }

  String _semanticLabel(FileDialogEntry entry) {
    final parts = [entry.name, widget.names.entryType(entry)];
    if (!isFolder(entry)) parts.add(shortSize(entry.size.toInt()));
    return parts.join(', ');
  }
}

/// Scrolling as usual, except while Ctrl is held: then the mouse wheel
/// zooms the items instead (see [FileDialogItemsViewState.zoom]).
class _ZoomablePhysics extends ScrollPhysics {
  const _ZoomablePhysics({super.parent});

  @override
  _ZoomablePhysics applyTo(ScrollPhysics? ancestor) =>
      _ZoomablePhysics(parent: buildParent(ancestor));

  @override
  bool shouldAcceptUserOffset(ScrollMetrics position) {
    final keys = HardwareKeyboard.instance;
    if (keys.isControlPressed || keys.isMetaPressed) return false;
    return super.shouldAcceptUserOffset(position);
  }
}

/// A details column's header.
String fileDialogColumnLabel(AppLocalizations s, FileDialogColumn column) =>
    switch (column) {
      FileDialogColumn.name => s.fileDialogName,
      FileDialogColumn.dateModified => s.fileDialogDateModified,
      FileDialogColumn.type => s.type,
      FileDialogColumn.size => s.fileDialogSize,
    };

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
    required this.onTap,
    required this.onLongPress,
    required this.child,
  });

  final String label;
  final bool selected;
  final bool focused;
  final bool row;
  final void Function(PointerDownEvent event) onPointerDown;
  final void Function(TapUpDetails details) onTap;
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
          onPointerDown: widget.onPointerDown,
          child: GestureDetector(
            onTapUp: widget.onTap,
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
                              end: _rowLead - 5,
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

/// The details header: a cell per column that sorts by it, a divider at
/// each column's end that resizes the column when dragged and fits it to
/// its contents when double-clicked, and File Explorer's header menu on a
/// right click.
class _DetailsHeader extends StatelessWidget {
  const _DetailsHeader({
    required this.onPointerDown,
    required this.columns,
    required this.sort,
    required this.label,
    required this.onSort,
    required this.onResizeStart,
    required this.onResize,
    required this.onFit,
    required this.onMenu,
  });

  /// Every press on the header, so the item area does not also take it
  /// for a press on its empty space.
  final void Function(PointerDownEvent event) onPointerDown;
  final List<(FileDialogColumn, double)> columns;
  final FileDialogSort sort;
  final String Function(FileDialogColumn column) label;
  final void Function(FileDialogColumn column) onSort;

  /// A column's divider started being dragged, the column [width] wide.
  final void Function(FileDialogColumn column, double width) onResizeStart;

  /// The divider is [distance] past where the drag started.
  final void Function(FileDialogColumn column, double distance) onResize;
  final void Function(FileDialogColumn column) onFit;
  final FileDialogColumnMenuCallback onMenu;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final secondary = theme.resources.textFillColorSecondary;

    Widget menuOn(FileDialogColumn? column, Widget child) => Listener(
      behavior: HitTestBehavior.opaque,
      onPointerDown: (event) {
        if (event.buttons == kSecondaryButton) onMenu(column, event.position);
      },
      child: child,
    );

    Widget header(FileDialogColumn column, double width) {
      final active = sort.column == column;
      final text = label(column);
      final cell = HoverButton(
        onPressed: () => onSort(column),
        semanticLabel: text,
        builder: (context, states) => Container(
          height: _rowHeight,
          color: ButtonThemeData.uncheckedInputColor(
            theme,
            states,
            transparentWhenNone: true,
          ),
          padding: const EdgeInsets.symmetric(horizontal: _cellPadding),
          child: Row(
            mainAxisAlignment: column == FileDialogColumn.size
                ? MainAxisAlignment.end
                : MainAxisAlignment.start,
            children: [
              Flexible(
                child: Text(
                  text,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(color: secondary),
                ),
              ),
              if (active) ...[
                const SizedBox(width: 4),
                Icon(
                  sort.ascending
                      ? Symbols.keyboard_arrow_up
                      : Symbols.keyboard_arrow_down,
                  size: 14,
                  color: secondary,
                ),
              ],
            ],
          ),
        ),
      );
      return SizedBox(
        width: width,
        child: menuOn(
          column,
          Stack(
            clipBehavior: Clip.none,
            children: [
              Positioned.fill(child: cell),
              // The divider on the column's end, as in File Explorer.
              PositionedDirectional(
                end: 0,
                top: 4,
                bottom: 4,
                width: 6,
                child: FileDialogDragHandle(
                  onStart: () => onResizeStart(column, width),
                  onDrag: (distance) => onResize(column, distance),
                  onDoubleTap: () => onFit(column),
                  child: Align(
                    alignment: AlignmentDirectional.centerEnd,
                    child: Container(
                      width: 1,
                      color: theme.resources.dividerStrokeColorDefault,
                    ),
                  ),
                ),
              ),
            ],
          ),
        ),
      );
    }

    return Listener(
      onPointerDown: onPointerDown,
      child: SizedBox(
        height: _rowHeight,
        child: Row(
          children: [
            // Lines up with the rows past their selection bar.
            menuOn(null, const SizedBox(width: _rowLead, height: _rowHeight)),
            for (final (column, width) in columns) header(column, width),
            Expanded(child: menuOn(null, const SizedBox.expand())),
          ],
        ),
      ),
    );
  }
}

class _DetailsRow extends StatelessWidget {
  const _DetailsRow({
    required this.entry,
    required this.names,
    required this.columns,
    required this.name,
    required this.cell,
  });

  final FileDialogEntry entry;
  final FileDialogNames names;
  final List<(FileDialogColumn, double)> columns;
  final Widget name;
  final String Function(FileDialogColumn column) cell;

  @override
  Widget build(BuildContext context) {
    final secondary = TextStyle(
      color: FluentTheme.of(context).resources.textFillColorSecondary,
    );
    return Opacity(
      opacity: entry.hidden ? 0.6 : 1,
      child: Row(
        children: [
          for (final (column, width) in columns)
            SizedBox(
              width: width,
              child: column == FileDialogColumn.name
                  ? Padding(
                      padding: const EdgeInsetsDirectional.only(
                        end: _cellPadding,
                      ),
                      child: Row(
                        children: [
                          FileDialogItemIcon(
                            entry: entry,
                            names: names,
                            size: 16,
                            thumbnails: null,
                          ),
                          const SizedBox(width: _nameIcon - 16),
                          Expanded(child: name),
                        ],
                      ),
                    )
                  : Padding(
                      padding: const EdgeInsets.symmetric(
                        horizontal: _cellPadding,
                      ),
                      child: Text(
                        cell(column),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        textAlign: column == FileDialogColumn.size
                            ? TextAlign.end
                            : TextAlign.start,
                        style: secondary,
                      ),
                    ),
            ),
          const Spacer(),
        ],
      ),
    );
  }
}

class _SmallRow extends StatelessWidget {
  const _SmallRow({
    required this.entry,
    required this.names,
    required this.name,
  });

  final FileDialogEntry entry;
  final FileDialogNames names;
  final Widget name;

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
          Expanded(child: name),
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
    required this.name,
  });

  final FileDialogEntry entry;
  final FileDialogNames names;
  final FileDialogThumbnails thumbnails;
  final double size;
  final Widget name;

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
            Expanded(child: name),
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
    required this.name,
  });

  final FileDialogEntry entry;
  final FileDialogNames names;
  final FileDialogThumbnails thumbnails;
  final Widget name;

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
                  name,
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
    required this.name,
    required this.wide,
  });

  final FileDialogEntry entry;
  final FileDialogNames names;
  final FileDialogThumbnails thumbnails;
  final String date;
  final Widget name;

  /// Whether there is room for the date and size beside the name.
  final bool wide;

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
                  name,
                  Text(names.entryType(entry), style: secondary, maxLines: 1),
                ],
              ),
            ),
            if (wide)
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

/// The name of an item being renamed, edited where it was shown: the text
/// stays where the name was and the box is drawn just outside it, as File
/// Explorer's is. The name without its extension is selected, Enter or
/// leaving the box keeps the new name, Escape drops it.
class _RenameField extends StatefulWidget {
  const _RenameField({
    super.key,
    required this.name,
    required this.selectStemOnly,
    required this.textAlign,
    required this.singleLine,
    required this.onCommit,
    required this.onCancel,
  });

  final String name;
  final bool selectStemOnly;
  final TextAlign textAlign;

  /// Rows keep the name on one line; icons let it wrap as their label does.
  final bool singleLine;
  final Future<bool> Function(String name) onCommit;
  final VoidCallback onCancel;

  @override
  State<_RenameField> createState() => _RenameFieldState();
}

class _RenameFieldState extends State<_RenameField>
    implements TextSelectionGestureDetectorBuilderDelegate {
  late final _text = TextEditingController(text: widget.name);
  final _focus = FocusNode(debugLabel: 'file dialog rename');
  final _editable = GlobalKey<EditableTextState>();
  late final _gestures = TextSelectionGestureDetectorBuilder(delegate: this);
  bool _committing = false;
  bool _cancelled = false;

  @override
  GlobalKey<EditableTextState> get editableTextKey => _editable;

  @override
  bool get forcePressEnabled => false;

  @override
  bool get selectionEnabled => true;

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
    final theme = FluentTheme.of(context);
    final style = DefaultTextStyle.of(context).style;
    final accent = theme.accentColor.defaultBrushFor(theme.brightness);
    final editable = EditableText(
      key: _editable,
      controller: _text,
      focusNode: _focus,
      style: style,
      textAlign: widget.textAlign,
      maxLines: widget.singleLine ? 1 : null,
      keyboardType: TextInputType.text,
      textInputAction: TextInputAction.done,
      onSubmitted: (_) => _commit(),
      autocorrect: false,
      enableSuggestions: false,
      cursorColor: style.color ?? theme.resources.textFillColorPrimary,
      cursorWidth: 1,
      backgroundCursorColor: theme.resources.textFillColorDisabled,
      selectionColor: accent.withValues(alpha: 0.4),
      selectionControls: fluentTextSelectionControls,
      rendererIgnoresPointer: true,
      mouseCursor: SystemMouseCursors.text,
    );
    return CallbackShortcuts(
      bindings: {
        const SingleActivator(LogicalKeyboardKey.escape): () {
          _cancelled = true;
          widget.onCancel();
        },
      },
      // As wide as the name, so the box hugs it; the box sits just outside
      // the text so the text does not move.
      child: IntrinsicWidth(
        child: Stack(
          clipBehavior: Clip.none,
          children: [
            Positioned(
              left: -3,
              right: -3,
              top: -2,
              bottom: -2,
              child: IgnorePointer(
                child: DecoratedBox(
                  decoration: BoxDecoration(
                    color: theme.resources.solidBackgroundFillColorBase,
                    border: Border.all(color: accent),
                    borderRadius: BorderRadius.circular(2),
                  ),
                ),
              ),
            ),
            _gestures.buildGestureDetector(
              behavior: HitTestBehavior.translucent,
              child: editable,
            ),
          ],
        ),
      ),
    );
  }
}
