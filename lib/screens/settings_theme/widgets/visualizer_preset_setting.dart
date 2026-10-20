import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';
import '../../../utils/rune_log.dart';
import '../../../utils/get_dir_path.dart';
import '../../../utils/visualizer_supported.dart';
import '../../../utils/visualizer_preset_dirs.dart';
import '../../../utils/file_storage/mac_secure_manager.dart';
import '../../../widgets/settings/settings_button.dart';
import '../../../widgets/settings/settings_tile_title.dart';
import '../../../bindings/bindings.dart';

/// The preset folders of the visualizer, managed like the library folders.
/// They are saved here and also sent to Rust, which is shared by all
/// visualizers of the app, so a running visualizer follows at once. Without
/// a folder the visualizer shows its default preset.
class VisualizerPresetSetting extends StatefulWidget {
  const VisualizerPresetSetting({super.key});

  @override
  State<VisualizerPresetSetting> createState() =>
      _VisualizerPresetSettingState();
}

class _VisualizerPresetSettingState extends State<VisualizerPresetSetting> {
  String selectedItem = '';
  List<String> _presetDirs = [];

  @override
  void initState() {
    super.initState();
    if (isVisualizerSupported) _load();
  }

  Future<void> _load() async {
    final dirs = await loadVisualizerPresetDirs();
    if (!mounted) return;
    setState(() => _presetDirs = dirs);
  }

  Future<void> _addFolder() async {
    final dir = await getDirPath();
    if (dir == null || !mounted) return;
    if (_presetDirs.contains(dir)) return;

    try {
      await MacSecureManager().saveBookmark(dir);
    } catch (e) {
      error$('Failed to keep access to $dir: $e');
    }
    await _update([..._presetDirs, dir]);
  }

  Future<void> _removeFolder(String dir) async {
    await _update(_presetDirs.where((x) => x != dir).toList());
  }

  Future<void> _update(List<String> dirs) async {
    await saveVisualizerPresetDirs(dirs);
    if (!mounted) return;
    setState(() => _presetDirs = dirs);

    // A visualizer that is open follows now; one that is not open reads the
    // setting when it starts.
    SetVisualizerPresetDirsRequest(paths: dirs).sendSignalToRust();
    final response = VisualizerStatusResponse.rustSignalStream.first.timeout(
      const Duration(seconds: 2),
    );
    VisualizerStatusRequest().sendSignalToRust();
    try {
      final status = (await response).message;
      if (status.running && status.presetCount > 0) {
        RandomVisualizerPresetRequest(
          transitionSeconds: 1.7,
        ).sendSignalToRust();
      }
    } on TimeoutException {
      // Without an answer the visualizer is not running.
    }
  }

  @override
  Widget build(BuildContext context) {
    if (!isVisualizerSupported) {
      return const SizedBox.shrink();
    }

    final s = S.of(context);

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SettingsButton(
          icon: Symbols.add,
          title: s.visualizerAddPresetFolder,
          subtitle: s.visualizerAddPresetFolderSubtitle,
          onPressed: _addFolder,
        ),
        const SizedBox(height: 2),
        SizedBox(
          width: double.maxFinite,
          child: ListView.builder(
            shrinkWrap: true,
            physics: const NeverScrollableScrollPhysics(),
            itemCount: _presetDirs.length,
            itemBuilder: (context, index) {
              final itemPath = _presetDirs[index];
              final isSelected = itemPath == selectedItem;
              final name = itemPath.split(RegExp(r'[/\\]')).lastWhere(
                    (x) => x.isNotEmpty,
                    orElse: () => itemPath,
                  );

              return ListTile.selectable(
                title: SettingsTileTitle(
                  icon: Symbols.folder,
                  title: name,
                  subtitle: itemPath,
                  showActions: isSelected,
                  actionsBuilder: (context) => Row(
                    children: [
                      Button(
                        onPressed: () => _removeFolder(itemPath),
                        child: Text(s.visualizerRemovePresetFolder),
                      ),
                    ],
                  ),
                ),
                selected: isSelected,
                onSelectionChange: (v) =>
                    setState(() => selectedItem = itemPath),
              );
            },
          ),
        ),
      ],
    );
  }
}
