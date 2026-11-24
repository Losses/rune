import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../../bindings/bindings.dart';
import '../../../l10n.dart';

import '../file_dialog_controller.dart';
import '../file_dialog_icons.dart';
import '../file_dialog_model.dart';
import '../file_dialog_names.dart';

/// The room after a pinned folder's pin: the 12 px Fluent's scroll bar
/// takes over the pane's edge while hovered, and a gap of 4 px.
const double _pinEndSpace = 16;

/// The navigation pane of the file dialog: Quick Access with the folders
/// pinned in the system's file browser, and This Device with the account's
/// folders and the drives. Every folder opens into its subfolders.
class FileDialogNavigationPane extends StatefulWidget {
  const FileDialogNavigationPane({
    super.key,
    required this.controller,
    required this.names,
  });

  final FileDialogController controller;
  final FileDialogNames names;

  @override
  State<FileDialogNavigationPane> createState() =>
      _FileDialogNavigationPaneState();
}

class _FileDialogNavigationPaneState extends State<FileDialogNavigationPane> {
  ListFileDialogPlacesResponse? _builtFrom;
  List<TreeViewItem> _items = const [];

  /// The items per location, the first one listed winning, so the
  /// highlighted item is the one highest in the pane.
  final _byLocation = <FileDialogLocation, TreeViewItem>{};

  @override
  void initState() {
    super.initState();
    widget.controller.addListener(_changed);
  }

  @override
  void didUpdateWidget(covariant FileDialogNavigationPane oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.controller != widget.controller) {
      oldWidget.controller.removeListener(_changed);
      widget.controller.addListener(_changed);
    }
  }

  @override
  void dispose() {
    widget.controller.removeListener(_changed);
    super.dispose();
  }

  void _changed() {
    if (mounted) setState(() {});
  }

  void _rebuildItems(ListFileDialogPlacesResponse places) {
    final names = widget.names;
    final s = S.of(context);
    _byLocation.clear();
    TreeViewItem folder(
      FileDialogLocation location,
      String name,
      String icon, {
      bool pinned = false,
    }) {
      final item = TreeViewItem(
        leading: FileDialogAssetIcon(icon, size: 16),
        content: Row(
          children: [
            Expanded(
              child: Text(name, maxLines: 1, overflow: TextOverflow.ellipsis),
            ),
            // Clear of the scroll bar that appears over the pane's edge.
            if (pinned)
              Padding(
                padding: const EdgeInsetsDirectional.only(
                  start: 8,
                  end: _pinEndSpace,
                ),
                child: Icon(
                  Symbols.keep,
                  size: 14,
                  color: FluentTheme.of(
                    context,
                  ).resources.textFillColorSecondary,
                ),
              ),
          ],
        ),
        semanticLabel: name,
        value: location,
        lazy: true,
        expanded: false,
        children: [],
        onExpandToggle: _loadChildren,
      );
      _byLocation.putIfAbsent(location, () => item);
      return item;
    }

    TreeViewItem group(String name, Widget icon, List<TreeViewItem> children) =>
        TreeViewItem(
          leading: icon,
          content: Text(name),
          semanticLabel: name,
          expanded: true,
          children: children,
        );

    _items = [
      if (places.pinned.isNotEmpty)
        group(
          s.fileDialogQuickAccess,
          // File Explorer marks Quick Access with a star.
          Icon(
            Symbols.star,
            size: 16,
            fill: 1,
            color: FluentTheme.of(
              context,
            ).accentColor.defaultBrushFor(FluentTheme.of(context).brightness),
          ),
          [
            for (final place in places.pinned)
              folder(
                place.location,
                names.place(place),
                names.folderIcon(place.location, 16),
                pinned: true,
              ),
          ],
        ),
      group(
        s.fileDialogThisDevice,
        FileDialogAssetIcon(thisDeviceIconAsset(16), size: 16),
        [
          for (final place in places.folders)
            folder(
              place.location,
              names.place(place),
              names.folderIcon(place.location, 16),
            ),
          for (final volume in places.volumes)
            folder(
              volume.location,
              names.volume(volume),
              volumeIconAsset(volume, 16),
            ),
        ],
      ),
    ];
    _builtFrom = places;
  }

  Future<void> _loadChildren(TreeViewItem item, bool expanding) async {
    final location = item.value;
    if (!expanding ||
        location is! FileDialogLocation ||
        item.children.isNotEmpty) {
      return;
    }
    List<FileDialogEntry> entries;
    try {
      entries = await widget.controller.backend.listFolder(location);
    } catch (_) {
      return;
    }
    final folders = sortEntries(
      entries.where(
        (entry) =>
            isFolder(entry) && (widget.controller.showHidden || !entry.hidden),
      ),
      const FileDialogSort(FileDialogColumn.name, true),
    );
    if (!mounted) return;
    setState(() {
      item.children.addAll([
        for (final entry in folders)
          TreeViewItem(
            leading: FileDialogAssetIcon(
              widget.names.folderIcon(entry.location, 16),
              size: 16,
            ),
            content: Text(
              entry.name,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            ),
            semanticLabel: entry.name,
            value: entry.location,
            lazy: true,
            expanded: false,
            children: [],
            onExpandToggle: _loadChildren,
          ),
      ]);
      _syncSelection();
    });
  }

  /// Highlight the item of the folder being shown.
  void _syncSelection() {
    final current = widget.controller.current;
    void visit(List<TreeViewItem> items) {
      for (final item in items) {
        item.selected = false;
        visit(item.children);
      }
    }

    visit(_items);
    final match = current == null ? null : _byLocation[current];
    if (match != null) {
      match.selected = true;
      return;
    }
    // A subfolder opened in the pane is highlighted when shown.
    TreeViewItem? find(List<TreeViewItem> items) {
      for (final item in items) {
        if (item.value == current) return item;
        final found = find(item.children);
        if (found != null) return found;
      }
      return null;
    }

    find(_items)?.selected = true;
  }

  @override
  Widget build(BuildContext context) {
    final places = widget.controller.places;
    if (places == null) return const SizedBox.shrink();
    if (!identical(places, _builtFrom)) _rebuildItems(places);
    _syncSelection();
    return Semantics(
      label: S.of(context).fileDialogNavigationPane,
      container: true,
      child: TreeView(
        items: _items,
        selectionMode: TreeViewSelectionMode.single,
        narrowSpacing: true,
        onItemInvoked: (item, reason) async {
          final location = item.value;
          if (reason == TreeViewItemInvokeReason.pressed &&
              location is FileDialogLocation) {
            await widget.controller.navigate(location);
          }
        },
      ),
    );
  }
}
