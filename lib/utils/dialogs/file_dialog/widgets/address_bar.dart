import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/services.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../../bindings/bindings.dart';
import '../../../l10n.dart';

import '../file_dialog_controller.dart';
import '../file_dialog_icons.dart';
import '../file_dialog_model.dart';
import '../file_dialog_names.dart';
import 'file_dialog_menu_button.dart';

const double _barHeight = 32;
const double _chevronWidth = 22;
const double _crumbPadding = 8;

/// The address bar of the file dialog, as in File Explorer: the folder's
/// path as segments with a chevron after each that lists its subfolders,
/// and, when clicked on its empty part or with Ctrl+L, Alt+D or F4, a text
/// box to type a path into. Its refresh button sits at its end, and stays
/// there while a path is typed.
class FileDialogAddressBar extends StatefulWidget {
  const FileDialogAddressBar({
    super.key,
    required this.controller,
    required this.names,
    required this.editFocus,
  });

  final FileDialogController controller;
  final FileDialogNames names;
  final FocusNode editFocus;

  @override
  State<FileDialogAddressBar> createState() => FileDialogAddressBarState();
}

class FileDialogAddressBarState extends State<FileDialogAddressBar> {
  final _text = TextEditingController();
  bool _editing = false;

  bool get editing => _editing;

  @override
  void initState() {
    super.initState();
    widget.editFocus.addListener(_focusChanged);
  }

  @override
  void didUpdateWidget(covariant FileDialogAddressBar oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.editFocus != widget.editFocus) {
      oldWidget.editFocus.removeListener(_focusChanged);
      widget.editFocus.addListener(_focusChanged);
    }
  }

  @override
  void dispose() {
    widget.editFocus.removeListener(_focusChanged);
    _text.dispose();
    super.dispose();
  }

  void _focusChanged() {
    // Leaving the box drops what was typed, as File Explorer does.
    if (!widget.editFocus.hasFocus && _editing) {
      setState(() => _editing = false);
    }
  }

  /// Switch to the text box with the whole path selected.
  void edit() {
    final location = widget.controller.current;
    if (location == null) return;
    _text.text = location.path;
    _text.selection = TextSelection(
      baseOffset: 0,
      extentOffset: _text.text.length,
    );
    setState(() => _editing = true);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) widget.editFocus.requestFocus();
    });
  }

  void _cancel() {
    setState(() => _editing = false);
    widget.editFocus.unfocus();
  }

  Future<void> _submit(String typed) async {
    setState(() => _editing = false);
    widget.editFocus.unfocus();
    await widget.controller.goTo(typed);
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    final refresh = _RefreshButton(controller: widget.controller);
    if (_editing) {
      return SizedBox(
        height: _barHeight,
        child: CallbackShortcuts(
          bindings: {const SingleActivator(LogicalKeyboardKey.escape): _cancel},
          child: TextBox(
            controller: _text,
            focusNode: widget.editFocus,
            placeholder: s.fileDialogAddress,
            onSubmitted: _submit,
            autocorrect: false,
            enableSuggestions: false,
            suffix: refresh,
          ),
        ),
      );
    }
    return Semantics(
      label: s.fileDialogAddress,
      container: true,
      child: _Crumbs(
        controller: widget.controller,
        names: widget.names,
        onEdit: edit,
        trailing: refresh,
      ),
    );
  }
}

/// Refreshes the folder shown. Clicking it leaves a path being typed in
/// the box, as buttons do not take the focus from a click.
class _RefreshButton extends StatelessWidget {
  const _RefreshButton({required this.controller});

  final FileDialogController controller;

  @override
  Widget build(BuildContext context) {
    final label = S.of(context).refresh;
    return Tooltip(
      message: '$label (F5)',
      child: Semantics(
        label: label,
        button: true,
        excludeSemantics: true,
        child: IconButton(
          style: const ButtonStyle(
            padding: WidgetStatePropertyAll(EdgeInsets.all(6)),
          ),
          icon: const Icon(Symbols.refresh, size: 16),
          onPressed: controller.current == null
              ? null
              : () => controller.refresh(),
        ),
      ),
    );
  }
}

class _Crumbs extends StatelessWidget {
  const _Crumbs({
    required this.controller,
    required this.names,
    required this.onEdit,
    required this.trailing,
  });

  final FileDialogController controller;
  final FileDialogNames names;
  final VoidCallback onEdit;
  final Widget trailing;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final location = controller.current;
    final style = theme.typography.body ?? const TextStyle(fontSize: 14);
    final chain = location == null
        ? <FileDialogLocation>[]
        : ancestry(location);

    return Container(
      height: _barHeight,
      decoration: BoxDecoration(
        color: theme.resources.controlFillColorDefault,
        borderRadius: BorderRadius.circular(4),
        border: Border.all(color: theme.resources.controlStrokeColorDefault),
      ),
      child: Row(
        children: [
          const SizedBox(width: 6),
          if (location != null)
            FileDialogAssetIcon(names.folderIcon(location, 16), size: 16),
          Expanded(
            child: LayoutBuilder(
              builder: (context, constraints) {
                final available = constraints.maxWidth - _chevronWidth * 2;
                final shown = _fitting(context, chain, style, available);
                final hidden = chain.sublist(0, chain.length - shown.length);
                return Row(
                  children: [
                    if (hidden.isNotEmpty)
                      _overflow(context, hidden)
                    else
                      _chevron(context, null, chain.firstOrNull),
                    for (var index = 0; index < shown.length; index++) ...[
                      _crumb(
                        context,
                        shown[index],
                        style,
                        // Only a last segment wider than the bar shortens.
                        flexible:
                            index == shown.length - 1 &&
                            _width(context, shown[index], style) > available,
                      ),
                      _chevron(
                        context,
                        shown[index],
                        index + 1 < shown.length ? shown[index + 1] : null,
                      ),
                    ],
                    // The empty part of the bar switches to typing a path.
                    Expanded(
                      child: GestureDetector(
                        behavior: HitTestBehavior.opaque,
                        onTap: onEdit,
                        child: const SizedBox(height: _barHeight),
                      ),
                    ),
                  ],
                );
              },
            ),
          ),
          trailing,
          const SizedBox(width: 2),
        ],
      ),
    );
  }

  /// The room a segment and its chevron take.
  double _width(
    BuildContext context,
    FileDialogLocation location,
    TextStyle style,
  ) {
    final painter = TextPainter(
      text: TextSpan(text: names.location(location), style: style),
      maxLines: 1,
      textDirection: Directionality.of(context),
      textScaler: MediaQuery.textScalerOf(context),
    )..layout();
    final width = painter.width + _crumbPadding * 2 + _chevronWidth + 4;
    painter.dispose();
    return width;
  }

  /// The segments that fit in [width], always keeping the last.
  List<FileDialogLocation> _fitting(
    BuildContext context,
    List<FileDialogLocation> chain,
    TextStyle style,
    double width,
  ) {
    var used = 0.0;
    var first = chain.length;
    for (var index = chain.length - 1; index >= 0; index--) {
      final needed = _width(context, chain[index], style);
      if (used + needed > width && first < chain.length) break;
      used += needed;
      first = index;
    }
    return chain.sublist(first);
  }

  Widget _crumb(
    BuildContext context,
    FileDialogLocation location,
    TextStyle style, {
    required bool flexible,
  }) {
    final name = names.location(location);
    final crumb = Semantics(
      button: true,
      label: name,
      excludeSemantics: true,
      child: IconButton(
        style: const ButtonStyle(
          padding: WidgetStatePropertyAll(
            EdgeInsets.symmetric(horizontal: _crumbPadding, vertical: 4),
          ),
        ),
        icon: Text(
          name,
          style: style,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
        ),
        onPressed: () => controller.navigate(location),
      ),
    );
    return flexible ? Flexible(child: crumb) : crumb;
  }

  /// The chevron after [parent] listing its subfolders, with [next], the
  /// one on the current path, in bold. Without a parent it lists the
  /// navigation pane's top places.
  Widget _chevron(
    BuildContext context,
    FileDialogLocation? parent,
    FileDialogLocation? next,
  ) {
    return FileDialogMenuButton(
      padding: const EdgeInsets.symmetric(horizontal: 3, vertical: 6),
      semanticLabel: parent == null ? null : names.location(parent),
      items: () async {
        final folders = parent == null
            ? _topPlaces()
            : await _subfolders(parent);
        return [
          for (final (location, label) in folders)
            MenuFlyoutItem(
              leading: FileDialogAssetIcon(
                names.folderIcon(location, 16),
                size: 16,
              ),
              text: Text(
                label,
                style: location == next
                    ? const TextStyle(fontWeight: FontWeight.w600)
                    : null,
              ),
              onPressed: () => controller.navigate(location),
            ),
        ];
      },
      child: const Icon(Symbols.chevron_right, size: 16),
    );
  }

  Widget _overflow(BuildContext context, List<FileDialogLocation> hidden) {
    return FileDialogMenuButton(
      padding: const EdgeInsets.symmetric(horizontal: 3, vertical: 6),
      items: () async => [
        for (final location in hidden.reversed)
          MenuFlyoutItem(
            leading: FileDialogAssetIcon(
              names.folderIcon(location, 16),
              size: 16,
            ),
            text: Text(names.location(location)),
            onPressed: () => controller.navigate(location),
          ),
      ],
      child: const Icon(Symbols.keyboard_double_arrow_left, size: 16),
    );
  }

  List<(FileDialogLocation, String)> _topPlaces() {
    final places = controller.places;
    if (places == null) return const [];
    // Narrow dialogs have no navigation pane: the first chevron stands in.
    return [
      for (final place in places.pinned) (place.location, names.place(place)),
      for (final place in places.folders) (place.location, names.place(place)),
      for (final volume in places.volumes)
        (volume.location, names.volume(volume)),
    ];
  }

  Future<List<(FileDialogLocation, String)>> _subfolders(
    FileDialogLocation parent,
  ) async {
    try {
      final entries = await controller.backend.listFolder(parent);
      final folders = sortEntries(
        entries.where(
          (entry) =>
              isFolder(entry) && (controller.showHidden || !entry.hidden),
        ),
        const FileDialogSort(FileDialogColumn.name, true),
      );
      return [for (final entry in folders) (entry.location, entry.name)];
    } catch (_) {
      return const [];
    }
  }
}
