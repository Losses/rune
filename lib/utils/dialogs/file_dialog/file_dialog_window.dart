import 'dart:async';

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
import 'widgets/file_dialog_menu_button.dart';
import 'widgets/items_view.dart';
import 'widgets/navigation_pane.dart';

/// Room left above and below the dialog's body for its title, footer and
/// the window around it.
const double _chromeHeight = 260;
const double _navigationPaneWidth = 220;

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
    final target = _menuTarget.currentContext?.findRenderObject() as RenderBox?;
    // Menus open on the dialog's own navigator.
    final navigator = _navigator.currentContext?.findRenderObject();
    if (target == null || navigator is! RenderBox) return;
    final position = navigator.globalToLocal(globalPosition);
    unawaited(
      _menu.showFlyout(
        position: position,
        builder: (context) => MenuFlyout(
          items: entry == null
              ? _folderMenu(context)
              : _itemMenu(context, entry),
        ),
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

  String _viewLabel(BuildContext context, FileDialogViewMode mode) {
    final s = S.of(context);
    return switch (mode) {
      FileDialogViewMode.extraLargeIcons => s.fileDialogExtraLargeIcons,
      FileDialogViewMode.largeIcons => s.fileDialogLargeIcons,
      FileDialogViewMode.mediumIcons => s.fileDialogMediumIcons,
      FileDialogViewMode.smallIcons => s.fileDialogSmallIcons,
      FileDialogViewMode.list => s.fileDialogList,
      FileDialogViewMode.details => s.fileDialogDetails,
      FileDialogViewMode.tiles => s.fileDialogTiles,
      FileDialogViewMode.content => s.fileDialogContent,
    };
  }

  static IconData _viewIcon(FileDialogViewMode mode) => switch (mode) {
    FileDialogViewMode.extraLargeIcons => Symbols.photo_size_select_actual,
    FileDialogViewMode.largeIcons => Symbols.grid_view,
    FileDialogViewMode.mediumIcons => Symbols.apps,
    FileDialogViewMode.smallIcons => Symbols.view_comfy,
    FileDialogViewMode.list => Symbols.view_column,
    FileDialogViewMode.details => Symbols.view_list,
    FileDialogViewMode.tiles => Symbols.view_module,
    FileDialogViewMode.content => Symbols.view_agenda,
  };

  /// A radio choice in a menu, marked with a dot as Windows 11 marks it.
  static Widget? _radio(bool chosen) =>
      chosen ? const Icon(Symbols.fiber_manual_record, size: 8, fill: 1) : null;

  List<MenuFlyoutItemBase> _viewItems(BuildContext context) {
    final s = S.of(context);
    return [
      for (final (index, mode) in FileDialogViewMode.values.indexed)
        MenuFlyoutItem(
          leading: Icon(_viewIcon(mode), size: 16),
          text: Text(_viewLabel(context, mode)),
          trailing:
              _radio(controller.viewMode == mode) ??
              _Accelerator('Ctrl+Shift+${index + 1}'),
          onPressed: () => controller.setViewMode(mode),
        ),
      const MenuFlyoutSeparator(),
      MenuFlyoutItem(
        leading: Icon(
          controller.showHidden
              ? Symbols.check_box
              : Symbols.check_box_outline_blank,
          size: 16,
        ),
        text: Text(s.fileDialogHiddenItems),
        onPressed: () => controller.setShowHidden(!controller.showHidden),
      ),
    ];
  }

  List<MenuFlyoutItemBase> _sortItems(BuildContext context) {
    final s = S.of(context);
    final sort = controller.sort;
    MenuFlyoutItem column(FileDialogSortColumn value, String label) =>
        MenuFlyoutItem(
          leading: _radio(sort.column == value) ?? const SizedBox(width: 8),
          text: Text(label),
          onPressed: () =>
              controller.setSort(FileDialogSort(value, sort.ascending)),
        );
    MenuFlyoutItem order(bool ascending, String label) => MenuFlyoutItem(
      leading: _radio(sort.ascending == ascending) ?? const SizedBox(width: 8),
      text: Text(label),
      onPressed: () =>
          controller.setSort(FileDialogSort(sort.column, ascending)),
    );
    return [
      column(FileDialogSortColumn.name, s.fileDialogName),
      column(FileDialogSortColumn.dateModified, s.fileDialogDateModified),
      column(FileDialogSortColumn.type, s.type),
      column(FileDialogSortColumn.size, s.fileDialogSize),
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
      for (final (index, mode) in FileDialogViewMode.values.indexed)
        SingleActivator(
          LogicalKeyboardKey(LogicalKeyboardKey.digit1.keyId + index),
          control: true,
          shift: true,
        ): () =>
            controller.setViewMode(mode),
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
        iconButton(
          s.refresh,
          Symbols.refresh,
          controller.current == null ? null : () => controller.refresh(),
        ),
        if (!narrow) ...[
          const SizedBox(width: 4),
          SizedBox(width: 220, child: search),
        ],
      ],
    );

    Widget menuButton(
      String label,
      IconData icon,
      List<MenuFlyoutItemBase> items,
    ) => narrow
        ? Tooltip(
            message: label,
            child: Semantics(
              label: label,
              button: true,
              child: DropDownButton(
                leading: Icon(icon, size: 16),
                items: items,
              ),
            ),
          )
        : DropDownButton(
            leading: Icon(icon, size: 16),
            title: Text(label),
            items: items,
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
        menuButton(s.sortBy, Symbols.sort, _sortItems(context)),
        const SizedBox(width: 8),
        menuButton(
          s.fileDialogView,
          _viewIcon(controller.viewMode),
          _viewItems(context),
        ),
      ],
    );

    final items = FileDialogItemsView(
      key: _items,
      controller: controller,
      names: names,
      thumbnails: _thumbnails,
      focusNode: _itemsFocus,
      onContextMenu: _openContextMenu,
    );

    final body = SizedBox(
      height: bodyHeight,
      child: Column(
        children: [
          toolbar,
          const SizedBox(height: 8),
          commands,
          const SizedBox(height: 8),
          Expanded(
            child: FlyoutTarget(
              key: _menuTarget,
              controller: _menu.controller,
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  if (!narrow) ...[
                    SizedBox(
                      width: _navigationPaneWidth,
                      child: FileDialogNavigationPane(
                        controller: controller,
                        names: names,
                      ),
                    ),
                    const Divider(direction: Axis.vertical),
                    const SizedBox(width: 4),
                  ],
                  Expanded(child: items),
                ],
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
        ResponsiveDialogActions(
          FilledButton(
            onPressed: controller.current == null
                ? null
                : () => controller.submit(),
            child: Text(okLabel),
          ),
          Button(onPressed: onCancel, child: Text(s.cancel)),
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
