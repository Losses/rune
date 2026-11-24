import 'dart:async';
import 'dart:math' as math;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:provider/provider.dart';

import '../../../bindings/bindings.dart';
import '../../../providers/responsive_providers.dart';
import '../../../config/shortcuts.dart' as config;
import '../../../widgets/no_shortcuts.dart';
import '../../../widgets/responsive_dialog_actions.dart';
import '../../l10n.dart';
import '../../router/router_aware_flyout_controller.dart';
import '../information/confirm.dart';
import '../information/error.dart';
import '../unavailable_dialog_on_band.dart';

import 'file_dialog.dart';
import 'file_dialog_controller.dart';
import 'file_dialog_model.dart';
import 'file_dialog_names.dart';
import 'file_dialog_thumbnails.dart';
import 'widgets/address_bar.dart';
import 'widgets/drag_handle.dart';
import 'widgets/file_dialog_menu_button.dart';
import 'widgets/items_view.dart';
import 'widgets/navigation_pane.dart';
import 'widgets/view_menu.dart';

/// Room left above and below the dialog's body for its title, footer and
/// the window around it.
const double _chromeHeight = 260;

/// Room between the title and the toolbar, added to the dialog's own.
const double _belowTitle = 12;

/// The navigation pane is dragged no narrower than this, and no wider than
/// half the dialog.
const double _narrowestPane = 140;

/// Rune's file dialog, laid out as the Windows 11 common item dialog: an
/// address bar with history buttons and search, a command row, the
/// navigation pane beside the items, and the file name and type at the
/// bottom.
class FileDialogWindow extends StatefulWidget {
  const FileDialogWindow({
    super.key,
    required this.options,
    required this.close,
    this.backend = const FileDialogBackend(),
    this.thumbnails,
  });

  final FileDialogOptions options;
  final void Function(FileDialogResult? result) close;
  final FileDialogBackend backend;

  /// Replaces the thumbnail loader, for tests.
  final FileDialogThumbnails? thumbnails;

  @override
  State<FileDialogWindow> createState() => FileDialogWindowState();
}

class FileDialogWindowState extends State<FileDialogWindow> {
  late final FileDialogController controller = FileDialogController(
    options: widget.options,
    backend: widget.backend,
    onAccept: (result) => widget.close(result),
    onNotice: _showNotice,
    confirmReplace: _confirmReplace,
  );
  late final FileDialogThumbnails _thumbnails =
      widget.thumbnails ?? FileDialogThumbnails();

  final _addressBar = GlobalKey<FileDialogAddressBarState>();
  final _items = GlobalKey<FileDialogItemsViewState>();
  final _addressFocus = FocusNode(debugLabel: 'file dialog address');
  final _searchFocus = FocusNode(debugLabel: 'file dialog search');
  final _itemsFocus = FocusNode(debugLabel: 'file dialog items');
  final _fileNameFocus = FocusNode(debugLabel: 'file dialog file name');
  final _menu = RouterAwareFlyoutController();
  final _menuTarget = GlobalKey();

  /// Everything the dialog opens (menus, drop-downs, message boxes) opens
  /// on this navigator, inside the dialog's keyboard barrier.
  final _navigator = GlobalKey<NavigatorState>();
  BuildContext? _pageContext;

  /// Hands the menus the dialog opens their routes.
  final _flyoutRoutes = FlyoutRouteObserver();

  /// The navigation pane's width when its edge started being dragged.
  double _paneAtDragStart = 0;

  @override
  void initState() {
    super.initState();
    unawaited(
      controller.start().then((_) {
        if (mounted && !_fileNameFocus.hasFocus) _initialFocus();
      }),
    );
  }

  /// Saving starts in the file name box; opening starts in the items.
  void _initialFocus() {
    if (controller.saves) {
      _fileNameFocus.requestFocus();
      final name = controller.fileName.text;
      controller.fileName.selection = TextSelection(
        baseOffset: 0,
        extentOffset: stemOf(name).length,
      );
    } else {
      _itemsFocus.requestFocus();
    }
  }

  @override
  void dispose() {
    controller.dispose();
    if (widget.thumbnails == null) _thumbnails.dispose();
    _addressFocus.dispose();
    _searchFocus.dispose();
    _itemsFocus.dispose();
    _fileNameFocus.dispose();
    _menu.dispose();
    super.dispose();
  }

  String _title(BuildContext context) {
    final s = S.of(context);
    return widget.options.title ??
        switch (widget.options.mode) {
          FileDialogMode.openFile ||
          FileDialogMode.openFiles => s.fileDialogOpenTitle,
          FileDialogMode.pickFolder => s.fileDialogFolderTitle,
          FileDialogMode.saveFile => s.fileDialogSaveTitle,
        };
  }

  String _okLabel(BuildContext context) {
    final s = S.of(context);
    return widget.options.okButtonLabel ??
        switch (widget.options.mode) {
          FileDialogMode.openFile ||
          FileDialogMode.openFiles => s.fileDialogOpen,
          FileDialogMode.pickFolder => s.fileDialogSelectFolder,
          FileDialogMode.saveFile => s.save,
        };
  }

  // Messages, shown as File Explorer's message boxes are: titled after the
  // dialog.

  Future<void> _showNotice(FileDialogNotice notice) async {
    if (!mounted) return;
    final s = S.of(context);
    final message = switch (notice) {
      FileDialogFailureNotice(:final error, :final subject) =>
        describeFileDialogError(s, error, subject),
      FileDialogFileNotFoundNotice(:final typed) => s.fileDialogFileNotFound(
        typed,
      ),
      FileDialogPathNotFoundNotice(:final typed) => s.fileDialogPathNotFound(
        typed,
      ),
      FileDialogFolderNotFoundNotice(:final typed) =>
        s.fileDialogFolderNotFound(typed),
    };
    await showErrorDialog(
      context: _pageContext ?? context,
      title: _title(context),
      errorMessage: message,
      useRootNavigator: false,
    );
    // Back where the person was typing a name.
    if (mounted && notice is! FileDialogFailureNotice) {
      _fileNameFocus.requestFocus();
    }
  }

  Future<bool> _confirmReplace(String name) async {
    final s = S.of(context);
    final replace = await showConfirmDialog(
      context: _pageContext ?? context,
      title: s.fileDialogConfirmSaveAs,
      subtitle: s.fileDialogReplacePrompt(name),
      yesLabel: s.fileDialogReplace,
      noLabel: s.cancel,
      useRootNavigator: false,
    );
    return replace ?? false;
  }

  // Commands.

  Future<void> _newFolder() async {
    await controller.createFolder(S.of(context).fileDialogNewFolder);
    _itemsFocus.requestFocus();
  }

  void _copyPath() {
    final selected = controller.selectedEntries;
    final paths = selected.isEmpty
        ? [controller.current?.path].whereType<String>()
        : selected.map((entry) => entry.location.path);
    final text = paths.length == 1
        ? paths.first
        : paths.map((path) => '"$path"').join('\n');
    if (text.isNotEmpty) {
      unawaited(Clipboard.setData(ClipboardData(text: text)));
    }
  }

  /// Open a context menu for an item, or for the folder.
  void _openContextMenu(FileDialogEntry? entry, Offset globalPosition) {
    _showMenu(
      globalPosition,
      (context) =>
          entry == null ? _folderMenu(context) : _itemMenu(context, entry),
    );
  }

  /// File Explorer's menu on the details header: fitting columns to their
  /// contents, and which columns show. Fitting one column is offered only
  /// on a column.
  void _openColumnMenu(FileDialogColumn? column, Offset globalPosition) {
    _showMenu(globalPosition, (context) {
      final s = S.of(context);
      return [
        if (column != null)
          MenuFlyoutItem(
            text: Text(s.fileDialogSizeColumnToFit),
            onPressed: () => _items.currentState?.sizeColumnToFit(column),
          ),
        MenuFlyoutItem(
          text: Text(s.fileDialogSizeAllColumnsToFit),
          onPressed: () => _items.currentState?.sizeAllColumnsToFit(),
        ),
        const MenuFlyoutSeparator(),
        for (final each in FileDialogColumn.values)
          MenuFlyoutItem(
            leading: _checkMark(!controller.hiddenColumns.contains(each)),
            text: Text(fileDialogColumnLabel(s, each)),
            // The name is always shown.
            onPressed: each == FileDialogColumn.name
                ? null
                : () => controller.setColumnShown(
                    each,
                    controller.hiddenColumns.contains(each),
                  ),
          ),
      ];
    });
  }

  void _showMenu(
    Offset globalPosition,
    List<MenuFlyoutItemBase> Function(BuildContext context) items,
  ) {
    final target = _menuTarget.currentContext?.findRenderObject() as RenderBox?;
    // Menus open on the dialog's own navigator.
    final navigator = _navigator.currentContext?.findRenderObject();
    if (target == null || navigator is! RenderBox) return;
    final position = navigator.globalToLocal(globalPosition);
    unawaited(
      _menu.showFlyout(
        position: position,
        builder: (context) => MenuFlyout(items: items(context)),
      ),
    );
  }

  List<MenuFlyoutItemBase> _itemMenu(
    BuildContext context,
    FileDialogEntry entry,
  ) {
    final s = S.of(context);
    final multiple = controller.selection.length > 1;
    final folder = isFolder(entry);
    return [
      MenuFlyoutItem(
        leading: Icon(folder ? Symbols.folder_open : Symbols.check, size: 16),
        text: Text(
          folder ? s.fileDialogOpen : _okLabel(context),
          style: const TextStyle(fontWeight: FontWeight.w600),
        ),
        trailing: const _Accelerator('Enter'),
        onPressed: () {
          if (multiple && !folder) {
            unawaited(controller.submit());
          } else {
            unawaited(controller.activate(entry));
          }
        },
      ),
      const MenuFlyoutSeparator(),
      MenuFlyoutItem(
        leading: const Icon(Symbols.content_copy, size: 16),
        text: Text(s.fileDialogCopyAsPath),
        trailing: const _Accelerator('Ctrl+Shift+C'),
        onPressed: _copyPath,
      ),
      if (!multiple)
        MenuFlyoutItem(
          leading: const Icon(Symbols.edit, size: 16),
          text: Text(s.fileDialogRename),
          trailing: const _Accelerator('F2'),
          onPressed: controller.startRename,
        ),
    ];
  }

  List<MenuFlyoutItemBase> _folderMenu(BuildContext context) {
    final s = S.of(context);
    return [
      MenuFlyoutSubItem(
        leading: const Icon(Symbols.grid_view, size: 16),
        text: Text(s.fileDialogView),
        items: (context) => _viewItems(context),
      ),
      MenuFlyoutSubItem(
        leading: const Icon(Symbols.sort, size: 16),
        text: Text(s.sortBy),
        items: (context) => _sortItems(context),
      ),
      MenuFlyoutItem(
        leading: const Icon(Symbols.refresh, size: 16),
        text: Text(s.refresh),
        trailing: const _Accelerator('F5'),
        onPressed: () => unawaited(controller.refresh()),
      ),
      const MenuFlyoutSeparator(),
      MenuFlyoutItem(
        leading: const Icon(Symbols.create_new_folder, size: 16),
        text: Text(s.fileDialogNewFolder),
        trailing: const _Accelerator('Ctrl+Shift+N'),
        onPressed: () => unawaited(_newFolder()),
      ),
      MenuFlyoutItem(
        leading: const Icon(Symbols.content_copy, size: 16),
        text: Text(s.fileDialogCopyAsPath),
        onPressed: _copyPath,
      ),
    ];
  }

  /// The leading column of a choice in a context menu, as WinUI's radio
  /// menu items draw it: a dot before the current choice, then its icon.
  static Widget _radioMark(bool chosen, {IconData? icon}) => Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      SizedBox(
        width: 12,
        child: chosen
            ? const Icon(Symbols.fiber_manual_record, size: 6, fill: 1)
            : null,
      ),
      if (icon != null) ...[const SizedBox(width: 4), Icon(icon, size: 16)],
    ],
  );

  /// The leading column of a setting that is on or off: a check mark when
  /// it is on.
  static Widget _checkMark(bool on) => SizedBox(
    width: 16,
    child: on ? const Icon(Symbols.check, size: 16) : null,
  );

  List<MenuFlyoutItemBase> _viewItems(BuildContext context) {
    final s = S.of(context);
    return [
      for (final (index, view) in FileDialogView.named.indexed)
        MenuFlyoutItem(
          leading: _radioMark(
            controller.view == view,
            icon: fileDialogViewIcon(view),
          ),
          text: Text(fileDialogViewLabel(context, view)),
          trailing: _Accelerator('Ctrl+Shift+${index + 1}'),
          onPressed: () => controller.setView(view),
        ),
      const MenuFlyoutSeparator(),
      MenuFlyoutItem(
        leading: _checkMark(controller.showHidden),
        text: Text(s.fileDialogHiddenItems),
        onPressed: () => controller.setShowHidden(!controller.showHidden),
      ),
    ];
  }

  List<MenuFlyoutItemBase> _sortItems(BuildContext context) {
    final s = S.of(context);
    final sort = controller.sort;
    MenuFlyoutItem order(bool ascending, String label) => MenuFlyoutItem(
      leading: _radioMark(sort.ascending == ascending),
      text: Text(label),
      onPressed: () =>
          controller.setSort(FileDialogSort(sort.column, ascending)),
    );
    return [
      for (final column in FileDialogColumn.values)
        MenuFlyoutItem(
          leading: _radioMark(sort.column == column),
          text: Text(fileDialogColumnLabel(s, column)),
          onPressed: () =>
              controller.setSort(FileDialogSort(column, sort.ascending)),
        ),
      const MenuFlyoutSeparator(),
      order(true, s.fileDialogAscending),
      order(false, s.fileDialogDescending),
    ];
  }

  // Keyboard.

  Map<ShortcutActivator, VoidCallback> _shortcuts() {
    void editAddress() => _addressBar.currentState?.edit();
    void focusSearch() => _searchFocus.requestFocus();
    return {
      const SingleActivator(LogicalKeyboardKey.keyL, control: true):
          editAddress,
      const SingleActivator(LogicalKeyboardKey.keyD, alt: true): editAddress,
      const SingleActivator(LogicalKeyboardKey.f4): editAddress,
      const SingleActivator(LogicalKeyboardKey.keyE, control: true):
          focusSearch,
      const SingleActivator(LogicalKeyboardKey.keyF, control: true):
          focusSearch,
      const SingleActivator(LogicalKeyboardKey.f3): focusSearch,
      const SingleActivator(LogicalKeyboardKey.arrowLeft, alt: true): () =>
          unawaited(controller.back()),
      const SingleActivator(LogicalKeyboardKey.arrowRight, alt: true): () =>
          unawaited(controller.forward()),
      const SingleActivator(LogicalKeyboardKey.arrowUp, alt: true): () =>
          unawaited(controller.up()),
      const SingleActivator(LogicalKeyboardKey.f5): () =>
          unawaited(controller.refresh()),
      const SingleActivator(LogicalKeyboardKey.keyR, control: true): () =>
          unawaited(controller.refresh()),
      const SingleActivator(
        LogicalKeyboardKey.keyN,
        control: true,
        shift: true,
      ): () =>
          unawaited(_newFolder()),
      const SingleActivator(
        LogicalKeyboardKey.keyC,
        control: true,
        shift: true,
      ): _copyPath,
      const SingleActivator(LogicalKeyboardKey.keyN, alt: true): () =>
          _fileNameFocus.requestFocus(),
      for (final (index, view) in FileDialogView.named.indexed)
        SingleActivator(
          LogicalKeyboardKey(LogicalKeyboardKey.digit1.keyId + index),
          control: true,
          shift: true,
        ): () =>
            controller.setView(view),
    };
  }

  // Layout.

  /// Rune's own shortcuts (playback, pages, Ctrl+O, Backspace and Escape)
  /// do nothing while the dialog or anything it opened has the keyboard.
  /// Inside text boxes [NoShortcuts] already leaves the keys to the text.
  static final Map<ShortcutActivator, Intent> _appShortcutsOff = {
    for (final activator in config.shortcuts.keys)
      activator: const DoNothingAndStopPropagationIntent(),
  };

  /// The mouse's back and forward buttons move through the folders, as in
  /// File Explorer. Rune's own handler for them finds the dialog unwilling
  /// to pop (see [PopScope] in [build]). While a notice or menu is open the
  /// press belongs to it: Rune's handler closes a notice, and the folder
  /// stays where it is.
  void _mouseButtons(PointerDownEvent event) {
    if (_navigator.currentState?.canPop() ?? false) return;
    if (event.buttons == kBackMouseButton) unawaited(controller.back());
    if (event.buttons == kForwardMouseButton) unawaited(controller.forward());
  }

  @override
  Widget build(BuildContext context) {
    return UnavailableDialogOnBand(
      $close: (_) => widget.close(null),
      // Back navigation (the mouse's back button, Android's back) does not
      // close the dialog; its buttons and Escape do.
      child: PopScope(
        canPop: false,
        child: Shortcuts(
          shortcuts: _appShortcutsOff,
          // Escape asks for DismissIntent. Menus and notices the dialog
          // opens dismiss themselves; on the dialog itself it closes it.
          child: NoShortcuts(
            CallbackShortcuts(
              bindings: _shortcuts(),
              child: FocusScope(
                autofocus: true,
                child: Listener(
                  onPointerDown: _mouseButtons,
                  // The frame around a dialog may scroll and so leave the
                  // height open; the navigator takes the height the frame
                  // lays dialogs out at.
                  child: LayoutBuilder(
                    builder: (context, constraints) => SizedBox(
                      width: constraints.maxWidth,
                      height: constraints.hasBoundedHeight
                          ? constraints.maxHeight
                          : constraints.minHeight,
                      child: Navigator(
                        key: _navigator,
                        observers: [_flyoutRoutes],
                        onGenerateRoute: (settings) => PageRouteBuilder<void>(
                          settings: settings,
                          opaque: false,
                          transitionDuration: Duration.zero,
                          reverseTransitionDuration: Duration.zero,
                          // Inside the page: the route maps DismissIntent to
                          // its own action, disabled for a page, and the
                          // nearest action is the one that answers.
                          pageBuilder: (context, animation, secondary) =>
                              Actions(
                                actions: <Type, Action<Intent>>{
                                  DismissIntent: CallbackAction<DismissIntent>(
                                    onInvoke: (_) {
                                      widget.close(null);
                                      return null;
                                    },
                                  ),
                                },
                                child: ListenableBuilder(
                                  listenable: controller,
                                  builder: (context, _) {
                                    _pageContext = context;
                                    return _dialog(context);
                                  },
                                ),
                              ),
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }

  Widget _dialog(BuildContext context) {
    final s = S.of(context);
    final responsive = Provider.of<ResponsiveProvider>(context);
    final narrow = responsive.smallerOrEqualTo(DeviceType.tablet);
    final names = FileDialogNames(s, controller.places);
    final height = MediaQuery.sizeOf(context).height;
    final bodyHeight =
        (height.clamp(dialogMinimumViewportHeight, double.infinity) -
                _chromeHeight)
            .clamp(240.0, 560.0);

    Widget iconButton(String tooltip, IconData icon, VoidCallback? onPressed) =>
        Tooltip(
          message: tooltip,
          child: Semantics(
            label: tooltip,
            button: true,
            excludeSemantics: true,
            child: IconButton(icon: Icon(icon, size: 16), onPressed: onPressed),
          ),
        );

    final search = TextBox(
      controller: controller.search,
      focusNode: _searchFocus,
      placeholder: controller.current == null
          ? s.search
          : s.fileDialogSearchIn(names.location(controller.current!)),
      onChanged: (_) => controller.searchChanged(),
      suffix: const Padding(
        padding: EdgeInsetsDirectional.only(end: 8),
        child: Icon(Symbols.search, size: 16),
      ),
    );

    // Narrow windows keep the path and the folder history in the first row
    // and move search beside the commands, which shrink to their icons.
    final toolbar = Row(
      children: [
        iconButton(
          s.back,
          Symbols.arrow_back,
          controller.canGoBack ? () => controller.back() : null,
        ),
        iconButton(
          s.fileDialogForward,
          Symbols.arrow_forward,
          controller.canGoForward ? () => controller.forward() : null,
        ),
        if (!narrow)
          FileDialogMenuButton(
            tooltip: s.fileDialogRecentLocations,
            padding: const EdgeInsets.symmetric(horizontal: 2, vertical: 6),
            items: () async {
              final history = controller.backHistory;
              return [
                for (var index = history.length - 1; index >= 0; index--)
                  MenuFlyoutItem(
                    text: Text(names.location(history[index])),
                    onPressed: () => controller.travelBack(index),
                  ),
              ];
            },
            child: const Icon(Symbols.keyboard_arrow_down, size: 16),
          ),
        iconButton(
          s.fileDialogUp,
          Symbols.arrow_upward,
          controller.canGoUp ? () => controller.up() : null,
        ),
        const SizedBox(width: 4),
        Expanded(
          child: FileDialogAddressBar(
            key: _addressBar,
            controller: controller,
            names: names,
            editFocus: _addressFocus,
          ),
        ),
        if (!narrow) ...[
          const SizedBox(width: 4),
          SizedBox(width: 220, child: search),
        ],
      ],
    );

    final newFolder = Button(
      onPressed: controller.hasEntries ? () => _newFolder() : null,
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          const Icon(Symbols.create_new_folder, size: 16),
          if (!narrow) ...[
            const SizedBox(width: 8),
            Text(s.fileDialogNewFolder),
          ],
        ],
      ),
    );

    final commands = Row(
      children: [
        narrow
            ? Tooltip(
                message: s.fileDialogNewFolder,
                child: Semantics(
                  label: s.fileDialogNewFolder,
                  button: true,
                  excludeSemantics: true,
                  child: newFolder,
                ),
              )
            : newFolder,
        if (narrow) ...[
          const SizedBox(width: 8),
          Expanded(child: search),
          const SizedBox(width: 8),
        ] else
          const Spacer(),
        _ViewButton(controller: controller),
      ],
    );

    final items = FileDialogItemsView(
      key: _items,
      controller: controller,
      names: names,
      thumbnails: _thumbnails,
      focusNode: _itemsFocus,
      onContextMenu: _openContextMenu,
      onColumnMenu: _openColumnMenu,
    );

    final body = SizedBox(
      height: bodyHeight,
      child: Column(
        children: [
          // Other dialogs follow their title with text; this one starts with
          // bordered controls, which need as much room below the title as
          // the dialog leaves above it.
          const SizedBox(height: _belowTitle),
          toolbar,
          const SizedBox(height: 8),
          commands,
          const SizedBox(height: 8),
          Expanded(
            child: FlyoutTarget(
              key: _menuTarget,
              controller: _menu.controller,
              child: LayoutBuilder(
                builder: (context, constraints) {
                  final width = constraints.maxWidth;
                  final widest = math.max(_narrowestPane, width / 2);
                  final pane = (width * controller.navigationShare).clamp(
                    _narrowestPane,
                    widest,
                  );
                  return Row(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      if (!narrow) ...[
                        SizedBox(
                          width: pane,
                          child: FileDialogNavigationPane(
                            controller: controller,
                            names: names,
                          ),
                        ),
                        // The line between the pane and the items, dragged
                        // to share the width as in File Explorer.
                        FileDialogDragHandle(
                          onStart: () => _paneAtDragStart = pane,
                          onDrag: (distance) => controller.setNavigationShare(
                            (_paneAtDragStart + distance).clamp(
                                  _narrowestPane,
                                  widest,
                                ) /
                                width,
                          ),
                          onEnd: () => controller.setNavigationShare(
                            controller.navigationShare,
                            save: true,
                          ),
                          child: const SizedBox(
                            width: 9,
                            child: Center(
                              child: Divider(direction: Axis.vertical),
                            ),
                          ),
                        ),
                      ],
                      Expanded(child: items),
                    ],
                  );
                },
              ),
            ),
          ),
        ],
      ),
    );

    return ContentDialog(
      constraints: BoxConstraints(maxWidth: narrow ? 420 : 1000),
      title: Column(
        children: [const SizedBox(height: 8), Text(_title(context))],
      ),
      content: body,
      actions: [
        _Footer(
          controller: controller,
          fileNameFocus: _fileNameFocus,
          okLabel: _okLabel(context),
          narrow: narrow,
          onCancel: () => widget.close(null),
        ),
      ],
    );
  }
}

/// The bottom of the dialog: the file name box, the file type box and the
/// buttons.
class _Footer extends StatelessWidget {
  static const _buttonWidth = BoxConstraints(minWidth: 120);

  const _Footer({
    required this.controller,
    required this.fileNameFocus,
    required this.okLabel,
    required this.narrow,
    required this.onCancel,
  });

  final FileDialogController controller;
  final FocusNode fileNameFocus;
  final String okLabel;

  /// In a narrow dialog the file type box takes a row of its own.
  final bool narrow;
  final VoidCallback onCancel;

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    final types = controller.options.fileTypes;
    final nameLabel = controller.picksFolders ? s.folder : s.fileDialogFileName;
    final nameBox = TextBox(
      controller: controller.fileName,
      focusNode: fileNameFocus,
      onSubmitted: (_) => controller.submit(),
      autocorrect: false,
      enableSuggestions: false,
    );
    final typeBox = types.isEmpty
        ? null
        : Semantics(
            label: controller.saves
                ? s.fileDialogSaveAsType
                : s.fileDialogFileTypes,
            child: ComboBox<int>(
              isExpanded: true,
              value: controller.fileTypeIndex,
              items: [
                for (final (index, type) in types.indexed)
                  ComboBoxItem(value: index, child: Text(type.display)),
              ],
              onChanged: (index) {
                if (index != null) controller.setFileType(index);
              },
            ),
          );

    Widget labelled(String label, Widget child) => Row(
      children: [
        SizedBox(
          width: 104,
          child: Text(label, maxLines: 1, overflow: TextOverflow.ellipsis),
        ),
        Expanded(child: child),
      ],
    );

    final ok = FilledButton(
      onPressed: controller.current == null ? null : () => controller.submit(),
      child: Text(okLabel),
    );
    final cancel = Button(onPressed: onCancel, child: Text(s.cancel));

    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (controller.saves) ...[
          labelled(nameLabel, Semantics(label: nameLabel, child: nameBox)),
          if (typeBox != null) ...[
            const SizedBox(height: 8),
            labelled(s.fileDialogSaveAsType, typeBox),
          ],
        ] else if (narrow) ...[
          labelled(nameLabel, Semantics(label: nameLabel, child: nameBox)),
          if (typeBox != null) ...[
            const SizedBox(height: 8),
            labelled('', typeBox),
          ],
        ] else
          labelled(
            nameLabel,
            Row(
              children: [
                Expanded(
                  child: Semantics(label: nameLabel, child: nameBox),
                ),
                if (typeBox != null) ...[
                  const SizedBox(width: 8),
                  SizedBox(width: 240, child: typeBox),
                ],
              ],
            ),
          ),
        const SizedBox(height: 12),
        if (narrow)
          ResponsiveDialogActions(ok, cancel)
        else
          // As in Windows: the buttons at the end, as wide as their labels
          // need, however wide the dialog is.
          Row(
            mainAxisAlignment: MainAxisAlignment.end,
            children: [
              ConstrainedBox(constraints: _buttonWidth, child: ok),
              const SizedBox(width: 8),
              ConstrainedBox(constraints: _buttonWidth, child: cancel),
            ],
          ),
      ],
    );
  }
}

/// A command's keyboard shortcut at the end of a menu item.
class _Accelerator extends StatelessWidget {
  const _Accelerator(this.keys);

  final String keys;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    return Padding(
      padding: const EdgeInsetsDirectional.only(start: 24),
      child: Text(
        keys,
        style: theme.typography.caption?.copyWith(
          color: theme.resources.textFillColorSecondary,
        ),
      ),
    );
  }
}

/// The view button, as in Windows' file dialogs: pressing it steps to the
/// next view, its arrow opens the view slider. Both halves have the fill
/// of the New Folder button beside it.
class _ViewButton extends StatelessWidget {
  const _ViewButton({required this.controller});

  final FileDialogController controller;

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    return SplitButton(
      onInvoked: () => controller.setView(controller.view.next),
      flyout: FileDialogViewMenu(controller: controller),
      secondaryBuilder: (context, showFlyout, flyoutController) => Tooltip(
        message: s.fileDialogMoreOptions,
        child: HoverButton(
          onPressed: showFlyout,
          semanticLabel: s.fileDialogMoreOptions,
          builder: (context, states) => FlyoutTarget(
            controller: flyoutController,
            child: Container(
              color: ButtonThemeData.buttonColor(
                context,
                flyoutController.isOpen ? {WidgetState.pressed} : states,
              ),
              padding: const EdgeInsets.symmetric(horizontal: 8),
              alignment: Alignment.center,
              child: const ChevronDown(),
            ),
          ),
        ),
      ),
      child: Tooltip(
        message: s.fileDialogChangeView,
        child: Semantics(
          label: s.fileDialogChangeView,
          button: true,
          excludeSemantics: true,
          // The split button paints this half only while it is hovered or
          // pressed; at rest it is clear, so the button's own fill is
          // painted here.
          child: Builder(
            builder: (context) {
              final states =
                  HoverButton.maybeOf(context)?.states ?? const <WidgetState>{};
              final resting =
                  !states.isHovered && !states.isPressed && !states.isDisabled;
              return Container(
                color: resting
                    ? ButtonThemeData.buttonColor(context, states)
                    : null,
                padding: const EdgeInsets.symmetric(
                  horizontal: 10,
                  vertical: 8,
                ),
                child: Icon(
                  fileDialogViewIcon(controller.view),
                  size: 16,
                  color: ButtonThemeData.buttonForegroundColor(context, states),
                ),
              );
            },
          ),
        ),
      ),
    );
  }
}
