import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';
import '../../../utils/router/navigation.dart';
import '../../../widgets/no_shortcuts.dart';
import '../../../widgets/settings/settings_tile_title.dart';
import '../../add_library_flows.dart';

enum AddLibrarySource { localFolder, runeDevice }

Future<void> showAddLibraryGuide(
  BuildContext context, {
  bool navigateIfFailed = false,
  Future<void> Function(BuildContext)? addLocalFolder,
  Future<void> Function(BuildContext)? addRuneDevice,
}) async {
  final source = await $showModal<AddLibrarySource>(
    context,
    (context, $close) => _AddLibraryGuideDialog($close: $close),
  );
  if (!context.mounted || source == null) return;

  switch (source) {
    case AddLibrarySource.localFolder:
      await (addLocalFolder ??
          (context) => addLocalFolderLibrary(
            context,
            navigateIfFailed: navigateIfFailed,
          ))(context);
    case AddLibrarySource.runeDevice:
      await (addRuneDevice ?? addRuneDeviceLibrary)(context);
  }
}

class _AddLibraryGuideDialog extends StatefulWidget {
  const _AddLibraryGuideDialog({required this.$close});

  final void Function(AddLibrarySource?) $close;

  @override
  State<_AddLibraryGuideDialog> createState() => _AddLibraryGuideDialogState();
}

class _AddLibraryGuideDialogState extends State<_AddLibraryGuideDialog> {
  AddLibrarySource? _selected;

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    return NoShortcuts(
      ContentDialog(
        title: Column(children: [SizedBox(height: 8), Text(s.addLibrary)]),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _sourceRow(
              AddLibrarySource.localFolder,
              Symbols.folder,
              s.localFolder,
              s.localFolderSubtitle,
            ),
            _sourceRow(
              AddLibrarySource.runeDevice,
              Symbols.devices,
              s.runeDevice,
              s.runeDeviceSubtitle,
            ),
          ],
        ),
        actions: [
          Button(onPressed: () => widget.$close(null), child: Text(s.cancel)),
        ],
      ),
    );
  }

  Widget _sourceRow(
    AddLibrarySource source,
    IconData icon,
    String title,
    String subtitle,
  ) {
    final selected = _selected == source;
    return ListTile.selectable(
      title: SettingsTileTitle(
        icon: icon,
        title: title,
        subtitle: subtitle,
        showActions: selected,
        actionsBuilder: (context) => Row(
          mainAxisAlignment: MainAxisAlignment.end,
          children: [
            if (selected)
              Button(
                onPressed: () => widget.$close(source),
                child: Text(S.of(context).confirm),
              ),
          ],
        ),
      ),
      selected: selected,
      onSelectionChange: (value) => setState(() {
        _selected = value ? source : null;
      }),
    );
  }
}
