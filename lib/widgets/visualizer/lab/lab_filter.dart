import 'dart:math' as math;

import 'package:flutter/services.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/router/navigation.dart';
import '../../../utils/router/router_aware_flyout_controller.dart';
import '../../ax_reveal/ax_reveal.dart';
import '../../context_menu_wrapper.dart';
import '../../hover_opacity.dart';
import '../../no_shortcuts.dart';
import 'lab_data.dart';
import 'lab_layout_mode.dart';
import 'lab_menu.dart';
import 'lab_rows.dart';
import 'lab_views.dart';

const _dialogWidth = 800.0;
// The rounding of fluent's dialog, which the panes inside it keep to.
const _dialogRadius = 12.0;
// The views' pane on the left, as Groove's: the views, and the room around
// them.
const _paneWidth = 232.0;
// The least width the list needs beside the pane. In a narrower dialog the
// pane is not beside the list but opens over it.
const _listMinWidth = 400.0;
// Where the list on the right starts and ends: the view's name and the
// headings start where the text of fluent's list items does (their margin
// and their room before the title), and their checkboxes end at the end.
const _listStart = 16.0;
const _listEnd = 16.0;
// The width of each strip's column, Plan and Actual: its name in each group's
// heading, and under it a checkbox on each row of the group.
const _stripColumn = 52.0;

// A group's heading, and the room before it that sets the group apart from
// the one before. The room scrolls with the list: were it part of the
// heading, it would stick with it, a band over the rows under the view's name.
const _headerHeight = 28.0;
const _headerGap = 20.0;

// Text that says less than what is beside it, as the app's settings write
// their subtitles.
TextStyle? _secondary(BuildContext context) {
  final theme = FluentTheme.of(context);
  return theme.typography.caption?.apply(
    color: theme.inactiveColor.withAlpha(160),
  );
}

// A row is one of fluent's list items with its name alone, as tall as it is
// with the buttons it shows on hover, so that it keeps its height then.
const _rowHeight = 48.0;

/// The columns of the list, by group in catalogue order. With [leavePinned]
/// the pinned columns are left out, for a list that shows them once, under
/// the pinned rows.
List<(String, List<LabColumn>)> labRowGroups(
  List<LabColumn> columns,
  LabRowSelection selection, {
  bool leavePinned = false,
}) {
  final out = <(String, List<LabColumn>)>[];
  for (final c in columns) {
    if (leavePinned && selection.isPinned(c.id)) continue;
    if (out.isEmpty || out.last.$1 != c.group) out.add((c.group, []));
    out.last.$2.add(c);
  }
  return out;
}

/// Opens the Rows dialog over the page: the views, and every row with what
/// it shows. It changes [model], so the timeline follows at once. The lab's
/// dark theme is passed on, as the dialog is outside the page. With [reveal],
/// the id of a column, the list opens at that row, marked. The dialog has no
/// title: the view on the right names what it shows.
Future<void> showLabRowsDialog(
  BuildContext context, {
  required LabRowsModel model,
  required List<LabColumn> columns,
  String? reveal,
}) {
  final theme = FluentTheme.of(context);
  final height = MediaQuery.sizeOf(context).height;
  final dialogHeight = math.min(math.max(420.0, height * 0.8), height);
  // On a phone the dialog is the window. Fluent's dialog already keeps to the
  // safe area, so its constraints fill that and its decoration has no
  // rounding.
  final full = labLayoutOf(context, listen: false).paged;

  return $showModal<void>(
    context,
    (context, close) => FluentTheme(
      data: theme,
      child: ContentDialog(
        constraints: full
            ? const BoxConstraints.expand()
            : BoxConstraints(
                maxWidth: _dialogWidth,
                minHeight: dialogHeight,
                maxHeight: dialogHeight,
              ),
        // The panes reach the dialog's edges, inside its rounding, and
        // bring their own materials: the dialog has none of its own, so that
        // the acrylic of the views' pane shows what is behind it.
        style: ContentDialogThemeData(
          padding: EdgeInsets.zero,
          bodyPadding: EdgeInsets.zero,
          decoration: BoxDecoration(
            borderRadius: full
                ? BorderRadius.zero
                : const BorderRadius.all(Radius.circular(_dialogRadius)),
          ),
        ),
        content: ClipRRect(
          borderRadius: BorderRadius.circular(full ? 0 : _dialogRadius),
          child: LabRowsDialog(
            model: model,
            columns: columns,
            reveal: reveal,
            onClose: () => close(null),
          ),
        ),
      ),
    ),
    barrierDismissible: true,
    dismissWithEsc: true,
    // Fluent's own transition leaves a dialog a little smaller than its
    // constraints, which would leave a gap around one that is the window.
    transitionBuilder: full
        ? (context, animation, _, child) =>
              FadeTransition(opacity: animation, child: child)
        : null,
  );
}

/// The inside of the Rows dialog, as Groove's pane and To-Do's list are: on
/// the left the views, and on the right the view's name and every row by
/// group, each with a round check.
class LabRowsDialog extends StatefulWidget {
  const LabRowsDialog({
    super.key,
    required this.model,
    required this.columns,
    this.reveal,
    this.onClose,
  });

  final LabRowsModel model;
  final List<LabColumn> columns;

  /// The id of the column whose row the list opens at, marked.
  final String? reveal;

  /// Closes the dialog, from the button at the top right.
  final VoidCallback? onClose;

  @override
  State<LabRowsDialog> createState() => _LabRowsDialogState();
}

class _LabRowsDialogState extends State<LabRowsDialog> {
  final _scroll = ScrollController();
  final _collapsed = <String>{};
  // The row the list opened at, marked until the rows change.
  late String? _marked = widget.reveal;
  // A view being named in its place in the pane: a new one, or the one of
  // this name being renamed.
  bool _creating = false;
  String? _renaming;
  // Whether the views' pane is open over the list, where it is not beside it.
  bool _paneOpen = false;

  LabRowsModel get _model => widget.model;
  List<LabColumn> get _columns => widget.columns;

  @override
  void initState() {
    super.initState();
    final id = widget.reveal;
    if (id != null) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _scrollTo(id));
    }
  }

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  // The pinned columns are listed under Pinned, so not again in a group.
  List<(String, List<LabColumn>)> _groups() =>
      labRowGroups(_columns, _model.selection, leavePinned: true);

  List<LabColumn> _pinned() {
    final byId = {for (final c in _columns) c.id: c};
    return [
      for (final id in _model.selection.pins)
        if (byId[id] != null) byId[id]!,
    ];
  }

  void _set(LabRowSelection next) {
    if (_marked != null) setState(() => _marked = null);
    _model.setSelection(next);
  }

  // Shows or hides [rows], each with every strip it has.
  void _setRows(List<LabColumn> rows, {required bool on}) {
    var next = _model.selection;
    for (final c in rows) {
      next = next.withVisible(c, on: on);
    }
    _set(next);
  }

  // The offset of the top of [id]'s row in the list, or null when it is not
  // in it.
  double? _offsetOf(String id) {
    var y = 0.0;
    final pinned = _pinned();
    if (pinned.isNotEmpty) {
      y += _headerHeight;
      final i = pinned.indexWhere((c) => c.id == id);
      if (i >= 0) return y + i * _rowHeight;
      y += pinned.length * _rowHeight;
    }
    var first = pinned.isEmpty;
    for (final (name, rows) in _groups()) {
      if (!first) y += _headerGap;
      first = false;
      y += _headerHeight;
      if (_collapsed.contains(name)) continue;
      final i = rows.indexWhere((c) => c.id == id);
      if (i >= 0) return y + i * _rowHeight;
      y += rows.length * _rowHeight;
    }
    return null;
  }

  // Brings [id]'s row to the middle of the list, as far as it scrolls.
  void _scrollTo(String id) {
    final top = _offsetOf(id);
    if (top == null || !mounted || !_scroll.hasClients) return;
    final position = _scroll.position;
    final to = top - (position.viewportDimension - _rowHeight) / 2;
    _scroll.jumpTo(to.clamp(0.0, position.maxScrollExtent));
  }

  // ---------- views ----------

  // A new view is named where New View is, as To-Do names a new list; a
  // view is renamed where its name is. No dialog over the dialog.
  void _create() => setState(() {
    _creating = true;
    _renaming = null;
  });

  void _rename(String from) => setState(() {
    _renaming = from;
    _creating = false;
  });

  void _stopNaming() {
    if (!mounted) return;
    setState(() {
      _creating = false;
      _renaming = null;
    });
  }

  Widget _nameField({String? from}) {
    final taken = [for (final v in _model.views(_columns)) v.name];
    return _ViewNameField(
      key: ValueKey(from ?? '+'),
      initial: from ?? '',
      error: (name) => labViewNameError(name, taken, except: from),
      onDone: (name) {
        if (from == null) {
          _model.saveView(name, _columns);
        } else {
          _model.renameView(from, name, _columns);
        }
        _stopNaming();
      },
      onCancel: _stopNaming,
    );
  }

  Widget _viewsPane() {
    final views = _model.views(_columns);
    final current = _model.currentName;
    final own = _model.ownViews;
    final edited = _model.isEdited(_columns);

    List<MenuFlyoutItemBase> viewItems(LabRowView v) => [
      MenuFlyoutItem(
        leading: const Icon(Symbols.edit),
        text: const Text('Rename'),
        onPressed: () => _rename(v.name),
      ),
      MenuFlyoutItem(
        leading: const Icon(Symbols.save),
        text: const Text('Update'),
        onPressed: () => _model.updateView(v.name),
      ),
      MenuFlyoutItem(
        leading: const Icon(Symbols.delete),
        text: const Text('Delete'),
        onPressed: () => _model.deleteView(v.name),
      ),
    ];

    Widget tile(LabRowView v) => _ViewTile(
      name: v.name,
      icon: _viewIcon(v),
      selected: v.name == current,
      onPressed: () {
        _model.applyView(v.name, _columns);
        if (_paneOpen) setState(() => _paneOpen = false);
      },
      // The view the rows follow, changed since, has the way back to it.
      onReset: v.name == current && edited
          ? () => _model.resetView(_columns)
          : null,
      contextItems: v.builtIn ? null : () => viewItems(v),
      menu: v.builtIn
          ? null
          : _MoreMenu(tooltip: 'More Options', items: () => viewItems(v)),
    );

    // As Groove's pane: on acrylic, which fluent's menus also use, the
    // views as fluent's selectable list items, a line, the user's own views
    // and a way to make one.
    return Acrylic(
      key: const ValueKey('lab-views-pane'),
      child: SizedBox(
        width: _paneWidth,
        child: ListView(
          padding: const EdgeInsets.symmetric(vertical: 4),
          children: [
            for (final v in views)
              if (v.builtIn) tile(v),
            const Padding(
              padding: EdgeInsets.symmetric(horizontal: 12, vertical: 8),
              child: Divider(
                style: DividerThemeData(
                  horizontalMargin: EdgeInsets.zero,
                  verticalMargin: EdgeInsets.zero,
                ),
              ),
            ),
            for (final v in own)
              if (v.name == _renaming) _nameField(from: v.name) else tile(v),
            if (_creating)
              _nameField()
            else
              AxReveal0(
                child: ListTile(
                  leading: const Icon(Symbols.add, size: 16),
                  title: const Text('New View'),
                  onPressed: _create,
                ),
              ),
          ],
        ),
      ),
    );
  }

  // The icon of a view: one for each built-in view, a bookmark for the
  // user's own.
  static IconData _viewIcon(LabRowView v) {
    if (!v.builtIn) return Symbols.bookmark;
    return switch (v.name) {
      'Overview' => Symbols.dashboard,
      'Director' => Symbols.videocam,
      'Music' => Symbols.music_note,
      'Inputs' => Symbols.instant_mix,
      _ => Symbols.view_list,
    };
  }

  // ---------- rows ----------

  // The top of the list, as To-Do's: the view's name, large, and the close
  // button at the right edge.
  // It is painted after the list, and its ground a pixel past its foot,
  // over the list's first pixel: the dialog's scale puts that edge between
  // two pixels, where the rows going under a stuck heading would show as a
  // seam. The pixel past it lies on the heading, which has the same ground.
  Widget _header({required bool overlay}) {
    final theme = FluentTheme.of(context);
    final close = widget.onClose;
    final ground = _listColor(theme);
    return DecoratedBox(
      decoration: BoxDecoration(
        color: ground,
        boxShadow: [BoxShadow(color: ground, offset: const Offset(0, 1))],
      ),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(_listStart, 14, _listEnd, 10),
        child: Row(
          children: [
            // Where the pane is not beside the list, the way to open it.
            if (overlay)
              Padding(
                padding: const EdgeInsetsDirectional.only(end: 6),
                child: Tooltip(
                  message: 'Views',
                  child: IconButton(
                    icon: const Icon(Symbols.menu, size: 18),
                    onPressed: () => setState(() => _paneOpen = true),
                  ),
                ),
              ),
            Expanded(
              child: Text(
                _model.currentName,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: theme.typography.subtitle,
              ),
            ),
            if (close != null)
              Tooltip(
                message: 'Close',
                child: IconButton(
                  icon: const Icon(Symbols.close, size: 18),
                  onPressed: close,
                ),
              ),
          ],
        ),
      ),
    );
  }

  Widget _row(
    LabColumn column, {
    bool highlighted = false,
    bool? first,
    bool? last,
  }) => _ColumnRow(
    column: column,
    selection: _model.selection,
    columns: _columns,
    highlighted: highlighted,
    onChanged: _set,
    pinnedFirst: first,
    pinnedLast: last,
  );

  Widget _list() {
    final groups = _groups();
    final pinned = _pinned();
    const gap = SliverToBoxAdapter(child: SizedBox(height: _headerGap));

    return CustomScrollView(
      controller: _scroll,
      slivers: [
        if (pinned.isNotEmpty)
          SliverMainAxisGroup(
            slivers: [
              SliverPersistentHeader(
                pinned: true,
                delegate: _GroupHeader(name: 'Pinned'),
              ),
              SliverList.builder(
                itemCount: pinned.length,
                itemBuilder: (context, i) => _row(
                  pinned[i],
                  highlighted: pinned[i].id == _marked,
                  first: i == 0,
                  last: i == pinned.length - 1,
                ),
              ),
            ],
          ),
        for (final (index, (name, rows)) in groups.indexed) ...[
          if (index > 0 || pinned.isNotEmpty) gap,
          SliverMainAxisGroup(
            slivers: [
              SliverPersistentHeader(
                pinned: true,
                delegate: _GroupHeader(
                  name: name,
                  collapsed: _collapsed.contains(name),
                  onShowAll: () => _setRows(rows, on: true),
                  onHideAll: () => _setRows(rows, on: false),
                  onCollapse: () => setState(() {
                    _collapsed.contains(name)
                        ? _collapsed.remove(name)
                        : _collapsed.add(name);
                  }),
                ),
              ),
              if (!_collapsed.contains(name))
                SliverList.builder(
                  itemCount: rows.length,
                  itemBuilder: (context, i) =>
                      _row(rows[i], highlighted: rows[i].id == _marked),
                ),
            ],
          ),
        ],
        if (groups.isEmpty && pinned.isEmpty)
          SliverToBoxAdapter(
            child: Padding(
              padding: const EdgeInsets.fromLTRB(_listStart, 16, _listEnd, 16),
              child: Text('No rows yet', style: _secondary(context)),
            ),
          ),
        const SliverToBoxAdapter(child: SizedBox(height: 8)),
      ],
    );
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final paged = labLayoutOf(context).paged;
    return ListenableBuilder(
      listenable: _model,
      builder: (context, _) => SizedBox(
        width: double.infinity,
        height: double.infinity,
        child: LayoutBuilder(
          builder: (context, box) {
            // On a phone upright the pane has no room beside the list, so it
            // opens over it, as fluent's navigation pane does when minimal.
            final overlay = paged && box.maxWidth < _paneWidth + _listMinWidth;
            // The list on a solid ground beside the pane, as Groove's
            // content is: the pane is the light, grained one.
            final list = DecoratedBox(
              decoration: BoxDecoration(color: _listColor(theme)),
              // The name above the list, laid out from the foot up so that
              // it is painted after the list, over its edge.
              child: Column(
                verticalDirection: VerticalDirection.up,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Expanded(child: _list()),
                  _header(overlay: overlay),
                ],
              ),
            );
            if (!overlay) {
              return Row(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  _viewsPane(),
                  Expanded(child: list),
                ],
              );
            }
            return Stack(
              children: [
                Positioned.fill(child: list),
                if (_paneOpen) ...[
                  // A tap outside the pane closes it.
                  Positioned.fill(
                    child: GestureDetector(
                      key: const ValueKey('lab-views-barrier'),
                      behavior: HitTestBehavior.opaque,
                      onTap: () => setState(() => _paneOpen = false),
                    ),
                  ),
                  Positioned(left: 0, top: 0, bottom: 0, child: _viewsPane()),
                ],
              ],
            );
          },
        ),
      ),
    );
  }
}

// The colour of the list's ground: fluent's solid base, as a window's own
// background is. It is opaque, so that a heading that stays at the top
// covers the rows under it.
Color _listColor(FluentThemeData theme) => theme.micaBackgroundColor;

/// A view in the pane on the left, as one of fluent's selectable list items:
/// its icon and name, and for a view of the user's the ⋯ menu on hover.
class _ViewTile extends StatefulWidget {
  const _ViewTile({
    required this.name,
    required this.icon,
    required this.selected,
    required this.onPressed,
    this.onReset,
    this.menu,
    this.contextItems,
  });

  final String name;
  final IconData icon;
  final bool selected;
  final VoidCallback onPressed;

  /// Brings the rows back to the view, when they were changed from it.
  final VoidCallback? onReset;
  final Widget? menu;

  /// The menu of a right click or a long press, as the ⋯ menu's: a finger
  /// never hovers, so it has no other way to the hover-only button.
  final List<MenuFlyoutItemBase> Function()? contextItems;

  @override
  State<_ViewTile> createState() => _ViewTileState();
}

class _ViewTileState extends State<_ViewTile> {
  bool _hover = false;
  final _menuController = RouterAwareFlyoutController();
  final _menuKey = GlobalKey();

  @override
  void dispose() {
    _menuController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final menu = widget.menu;
    final reset = widget.onReset;
    final items = widget.contextItems;
    // The app's reveal: a light that follows the pointer over the item.
    Widget tile = AxReveal0(
      child: ListTile.selectable(
        selected: widget.selected,
        onSelectionChange: (_) => widget.onPressed(),
        leading: Icon(widget.icon, size: 16),
        title: Text(widget.name, maxLines: 1, overflow: TextOverflow.ellipsis),
        trailing: reset == null && menu == null
            ? null
            : Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  if (reset != null)
                    Tooltip(
                      message: 'Reset View',
                      // fluent's small icon button, which keeps the item as
                      // tall as the others.
                      child: IconButton(
                        iconButtonMode: IconButtonMode.small,
                        icon: const Icon(Symbols.undo, size: 16),
                        onPressed: reset,
                      ),
                    ),
                  // Kept in the tree, so that its menu stays open when the
                  // pointer leaves the view.
                  if (menu != null)
                    Padding(
                      padding: const EdgeInsets.only(left: 4),
                      child: IgnorePointer(
                        ignoring: !_hover,
                        child: Opacity(opacity: _hover ? 1 : 0, child: menu),
                      ),
                    ),
                ],
              ),
      ),
    );
    if (items != null) {
      tile = _contextMenu(
        context,
        key: _menuKey,
        controller: _menuController,
        items: items,
        child: tile,
      );
    }
    return MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: tile,
    );
  }
}

/// Wraps [child] in the app's own context menu: a right click on desktop, a
/// long press elsewhere. It shows [items] at the pointer.
Widget _contextMenu(
  BuildContext context, {
  required GlobalKey key,
  required RouterAwareFlyoutController controller,
  required List<MenuFlyoutItemBase> Function() items,
  required Widget child,
}) {
  return ContextMenuWrapper(
    contextAttachKey: key,
    contextController: controller,
    onMiddleClick: (_) {},
    onContextMenu: (local) {
      final box = key.currentContext?.findRenderObject();
      if (box is! RenderBox) return;
      showLabFlyout<void>(
        context,
        controller,
        position: box.localToGlobal(local),
        builder: (context) => MenuFlyout(items: items()),
      );
    },
    child: child,
  );
}

/// A ⋯ button with the menu that [items] makes.
class _MoreMenu extends StatefulWidget {
  const _MoreMenu({required this.tooltip, required this.items});

  final String tooltip;
  final List<MenuFlyoutItemBase> Function() items;

  @override
  State<_MoreMenu> createState() => _MoreMenuState();
}

class _MoreMenuState extends State<_MoreMenu> {
  final _controller = RouterAwareFlyoutController();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  void _open() {
    showLabFlyout<void>(
      context,
      _controller,
      placementMode: FlyoutPlacementMode.bottomRight,
      builder: (context) => MenuFlyout(items: widget.items()),
    );
  }

  @override
  Widget build(BuildContext context) {
    return FlyoutTarget(
      controller: _controller.controller,
      child: Tooltip(
        message: widget.tooltip,
        child: IconButton(
          // Small, as it stands in a view's item, which it keeps as tall as
          // the others.
          iconButtonMode: IconButtonMode.small,
          icon: const Icon(Symbols.more_horiz, size: 16),
          onPressed: _open,
        ),
      ),
    );
  }
}

/// The name of a view typed in its place in the pane, as tall as an item:
/// Enter keeps it, unless [error] has something to say, which shows under
/// the box; Escape, or a click elsewhere, leaves things as they were.
class _ViewNameField extends StatefulWidget {
  const _ViewNameField({
    super.key,
    required this.initial,
    required this.error,
    required this.onDone,
    required this.onCancel,
  });

  final String initial;
  final String? Function(String name) error;
  final ValueChanged<String> onDone;
  final VoidCallback onCancel;

  @override
  State<_ViewNameField> createState() => _ViewNameFieldState();
}

class _ViewNameFieldState extends State<_ViewNameField> {
  late final _text = TextEditingController(text: widget.initial)
    ..selection = TextSelection(
      baseOffset: 0,
      extentOffset: widget.initial.length,
    );
  final _focus = FocusNode();
  final _field = GlobalKey<FormFieldState<String>>();
  // Whether a name was given: from then on, the box says what is wrong with
  // it as it is typed.
  bool _tried = false;
  bool _done = false;

  @override
  void initState() {
    super.initState();
    _focus.addListener(_onFocus);
  }

  @override
  void dispose() {
    _focus.removeListener(_onFocus);
    _focus.dispose();
    _text.dispose();
    super.dispose();
  }

  // A click elsewhere leaves it.
  void _onFocus() {
    if (!_focus.hasFocus) _cancel();
  }

  void _cancel() {
    if (_done) return;
    _done = true;
    widget.onCancel();
  }

  void _submit() {
    if (!_tried) setState(() => _tried = true);
    if (!(_field.currentState?.validate() ?? false)) {
      _focus.requestFocus();
      return;
    }
    _done = true;
    widget.onDone(_text.text.trim());
  }

  @override
  Widget build(BuildContext context) {
    // As the app's text boxes in dialogs: a key typed in it, such as a
    // space, is the box's, not a shortcut of the app's. Escape is the box's
    // too, so that it leaves the name and not the dialog.
    return NoShortcuts(
      Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
        child: Focus(
          onKeyEvent: (node, event) {
            if (event is KeyDownEvent &&
                event.logicalKey == LogicalKeyboardKey.escape) {
              _cancel();
              return KeyEventResult.handled;
            }
            return KeyEventResult.ignored;
          },
          child: TextFormBox(
            key: _field,
            controller: _text,
            focusNode: _focus,
            autofocus: true,
            placeholder: 'View Name',
            autovalidateMode: _tried
                ? AutovalidateMode.onUserInteraction
                : AutovalidateMode.disabled,
            validator: (name) => widget.error(name ?? ''),
            onFieldSubmitted: (_) => _submit(),
          ),
        ),
      ),
    );
  }
}

/// A sticky heading: a group's, which collapses it on a click and offers
/// Show All and Hide All while the pointer is on it, or the pinned rows'.
class _GroupHeader extends SliverPersistentHeaderDelegate {
  _GroupHeader({
    required this.name,
    this.collapsed = false,
    this.onShowAll,
    this.onHideAll,
    this.onCollapse,
  });

  final String name;
  final bool collapsed;
  final VoidCallback? onShowAll;
  final VoidCallback? onHideAll;
  final VoidCallback? onCollapse;

  @override
  double get minExtent => _headerHeight;

  @override
  double get maxExtent => _headerHeight;

  @override
  Widget build(BuildContext context, double shrinkOffset, bool overlaps) =>
      _GroupHeaderBar(this);

  @override
  bool shouldRebuild(_GroupHeader old) => true;
}

class _GroupHeaderBar extends StatefulWidget {
  const _GroupHeaderBar(this.header);

  final _GroupHeader header;

  @override
  State<_GroupHeaderBar> createState() => _GroupHeaderBarState();
}

class _GroupHeaderBarState extends State<_GroupHeaderBar> {
  bool _hover = false;
  final _menuController = RouterAwareFlyoutController();
  final _menuKey = GlobalKey();

  @override
  void dispose() {
    _menuController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final h = widget.header;
    // As the app's settings head a block: the name a little larger than the
    // body. It gives way first, where there is little room.
    Widget title = Row(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.baseline,
      textBaseline: TextBaseline.alphabetic,
      children: [
        if (h.collapsed) ...[
          const Icon(Symbols.chevron_right, size: 16),
          const SizedBox(width: 4),
        ],
        Flexible(
          child: Text(
            h.name,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: theme.typography.body?.apply(fontSizeFactor: 1.1),
          ),
        ),
      ],
    );
    // As the app's own group titles (the library's home): a little faded
    // until the pointer is on it, and a tap collapses the group.
    if (h.onCollapse != null) {
      title = GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: h.onCollapse,
        child: title,
      );
    }
    title = HoverOpacity(child: title);
    // The strips' names stand over the checkboxes of the group's rows, as a
    // list's column headers do, and stay with them as the heading sticks.
    Widget column(String name) => SizedBox(
      width: _stripColumn,
      child: Text(
        name,
        textAlign: TextAlign.center,
        style: _secondary(context),
      ),
    );
    Widget bar = Container(
      color: _listColor(theme),
      padding: const EdgeInsets.fromLTRB(_listStart, 0, _listEnd, 0),
      alignment: Alignment.centerLeft,
      child: Row(
        children: [
          Expanded(
            child: Row(
              children: [
                Flexible(child: title),
                if (_hover && h.onShowAll != null) ...[
                  const SizedBox(width: 8),
                  HyperlinkButton(
                    onPressed: h.onShowAll,
                    child: const Text('Show All'),
                  ),
                  HyperlinkButton(
                    onPressed: h.onHideAll,
                    child: const Text('Hide All'),
                  ),
                ],
              ],
            ),
          ),
          if (!h.collapsed) ...[column('Plan'), column('Actual')],
        ],
      ),
    );
    if (h.onShowAll != null) {
      // The same two, for a finger.
      bar = _contextMenu(
        context,
        key: _menuKey,
        controller: _menuController,
        items: () => [
          MenuFlyoutItem(text: const Text('Show All'), onPressed: h.onShowAll),
          MenuFlyoutItem(text: const Text('Hide All'), onPressed: h.onHideAll),
        ],
        child: bar,
      );
    }
    return MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: bar,
    );
  }
}

/// A row of the list, as one of fluent's list items: its name, a checkbox
/// for each of its strips, and on hover the pin, or for a pinned row the
/// moves and unpin. A click on the row shows or hides all of it.
class _ColumnRow extends StatefulWidget {
  const _ColumnRow({
    required this.column,
    required this.selection,
    required this.columns,
    required this.highlighted,
    required this.onChanged,
    this.pinnedFirst,
    this.pinnedLast,
  });

  final LabColumn column;
  final LabRowSelection selection;
  final List<LabColumn> columns;
  final bool highlighted;
  final ValueChanged<LabRowSelection> onChanged;

  /// Whether the row is the first or the last of the pinned rows. Null for a
  /// row that is not in that list.
  final bool? pinnedFirst;
  final bool? pinnedLast;

  @override
  State<_ColumnRow> createState() => _ColumnRowState();
}

class _ColumnRowState extends State<_ColumnRow> {
  bool _hover = false;
  final _menuController = RouterAwareFlyoutController();
  final _menuKey = GlobalKey();

  @override
  void dispose() {
    _menuController.dispose();
    super.dispose();
  }

  void _showRelated(List<LabColumn> related) {
    var next = _selection;
    for (final c in related) {
      next = next.withVisible(c, on: true);
    }
    widget.onChanged(next);
  }

  // What the hover buttons do, as a menu for a finger.
  List<MenuFlyoutItemBase> _menuItems() {
    final id = _column.id;
    final pinned = _selection.isPinned(id);
    final related = LabRowSelection.related(_column, widget.columns);
    return [
      if (widget.pinnedFirst != null) ...[
        if (!widget.pinnedFirst!)
          MenuFlyoutItem(
            leading: const Icon(Symbols.arrow_upward),
            text: const Text('Move Up'),
            onPressed: () => widget.onChanged(_selection.movePin(id, -1)),
          ),
        if (!widget.pinnedLast!)
          MenuFlyoutItem(
            leading: const Icon(Symbols.arrow_downward),
            text: const Text('Move Down'),
            onPressed: () => widget.onChanged(_selection.movePin(id, 1)),
          ),
      ] else if (related.isNotEmpty)
        MenuFlyoutItem(
          leading: const Icon(Symbols.link),
          text: const Text('Show Related Rows'),
          onPressed: () => _showRelated(related),
        ),
      MenuFlyoutItem(
        leading: Icon(pinned ? Symbols.keep_off : Symbols.push_pin),
        text: Text(pinned ? 'Unpin' : 'Pin to Top'),
        onPressed: () => widget.onChanged(
          pinned ? _selection.unpin(id) : _selection.pin(id),
        ),
      ),
    ];
  }

  LabColumn get _column => widget.column;
  LabRowSelection get _selection => widget.selection;

  // A strip's checkbox: on while the strip is, or none to tick, with why in
  // its tip, when the row has no such strip.
  // Its column's name above the list says what it is; a screen reader hears
  // it from the box itself.
  Widget _strip(
    String label, {
    required bool? on,
    String? missing,
    ValueChanged<bool>? onChanged,
  }) {
    final box = Checkbox(
      checked: on ?? false,
      onChanged: on == null ? null : (v) => onChanged!(v ?? false),
      semanticLabel: label,
    );
    return SizedBox(
      width: _stripColumn,
      child: Center(
        child: missing == null ? box : Tooltip(message: missing, child: box),
      ),
    );
  }

  Widget _planBox() => switch (_column.plan) {
    LabPlanAvailability.no => _strip(
      'Plan',
      on: null,
      missing: 'No Plan for Live Audio',
    ),
    LabPlanAvailability.yes => _strip(
      'Plan',
      on: _selection.planOn(_column.id),
      onChanged: (v) =>
          widget.onChanged(_selection.setPart(_column.id, plan: v)),
    ),
  };

  Widget _actualBox() => _column.hasActual
      ? _strip(
          'Actual',
          on: _selection.actualOn(_column.id),
          onChanged: (v) =>
              widget.onChanged(_selection.setPart(_column.id, actual: v)),
        )
      : _strip(
          'Actual',
          on: null,
          missing: _column.kind == LabColumnKind.sections
              ? 'One Strip for Sections'
              : 'No Actual Strip',
        );

  // An action of the row, as fluent sizes an icon button, a little apart
  // from the next.
  Widget _action(
    String tip,
    IconData icon,
    VoidCallback onPressed, {
    double fill = 0,
  }) => Padding(
    padding: const EdgeInsets.only(left: 4),
    child: Tooltip(
      message: tip,
      child: IconButton(
        icon: Icon(icon, size: 16, fill: fill),
        onPressed: onPressed,
      ),
    ),
  );

  List<Widget> _actions() {
    final id = _column.id;
    final pinned = _selection.isPinned(id);
    if (widget.pinnedFirst != null) {
      if (!_hover) return const [];
      // Only the moves it can make: the first goes no higher, the last no
      // lower.
      return [
        if (!widget.pinnedFirst!)
          _action(
            'Move Up',
            Symbols.arrow_upward,
            () => widget.onChanged(_selection.movePin(id, -1)),
          ),
        if (!widget.pinnedLast!)
          _action(
            'Move Down',
            Symbols.arrow_downward,
            () => widget.onChanged(_selection.movePin(id, 1)),
          ),
        _action(
          'Unpin',
          Symbols.keep_off,
          () => widget.onChanged(_selection.unpin(id)),
        ),
      ];
    }
    final related = LabRowSelection.related(_column, widget.columns);
    return [
      if (_hover && related.isNotEmpty)
        _action('Show Related Rows', Symbols.link, () => _showRelated(related)),
      if (_hover || pinned)
        _action(
          pinned ? 'Unpin' : 'Pin to Top',
          Symbols.push_pin,
          () => widget.onChanged(
            pinned ? _selection.unpin(id) : _selection.pin(id),
          ),
          fill: pinned ? 1 : 0,
        ),
    ];
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final shown = _selection.isShown(_column);

    final row = SizedBox(
      height: _rowHeight,
      child: ListTile(
        // The row a list opened at is marked as the one under the pointer.
        tileColor: widget.highlighted
            ? WidgetStateProperty.resolveWith(
                (states) => ButtonThemeData.uncheckedInputColor(theme, {
                  ...states,
                  WidgetState.hovered,
                }),
              )
            : null,
        title: Text(_column.name, maxLines: 1, overflow: TextOverflow.ellipsis),
        // The actions show on hover, before the strips, so that the
        // strips stay at the right edge.
        trailing: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            ..._actions(),
            const SizedBox(width: 8),
            _planBox(),
            _actualBox(),
          ],
        ),
        onPressed: () =>
            widget.onChanged(_selection.withVisible(_column, on: !shown)),
      ),
    );
    return MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: _contextMenu(
        context,
        key: _menuKey,
        controller: _menuController,
        items: _menuItems,
        child: row,
      ),
    );
  }
}
