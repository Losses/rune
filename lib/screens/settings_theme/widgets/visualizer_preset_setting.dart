import 'dart:io';
import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';
import '../../../utils/rune_log.dart';
import '../../../utils/get_dir_path.dart';
import '../../../utils/file_storage/mac_secure_manager.dart';
import '../../../widgets/settings/settings_button.dart';
import '../../../widgets/settings/settings_box_combo_box.dart';
import '../../../constants/configurations.dart';
import '../../../constants/settings_manager.dart';
import '../../../bindings/bindings.dart';

/// The preset source and folder of the visualizer. They are only saved here;
/// the visualizer page applies them when it is opened.
class VisualizerPresetSetting extends StatefulWidget {
  const VisualizerPresetSetting({super.key});

  @override
  State<VisualizerPresetSetting> createState() =>
      _VisualizerPresetSettingState();
}

class _VisualizerPresetSettingState extends State<VisualizerPresetSetting> {
  bool _builtinAvailable = false;
  String? _presetDir;
  String? _source;

  @override
  void initState() {
    super.initState();
    if (Platform.isMacOS) _load();
  }

  Future<void> _load() async {
    final dir = await $settingsManager.getValue<String>(
      kVisualizerPresetDirKey,
    );
    final source = await $settingsManager.getValue<String>(
      kVisualizerSourceKey,
    );
    if (!mounted) return;
    setState(() {
      _presetDir = dir != null && dir.isNotEmpty ? dir : null;
      _source = source;
    });

    // The pack is only known to Rust, which answers even if the visualizer
    // is not running.
    final response = VisualizerStatusResponse.rustSignalStream.first.timeout(
      const Duration(seconds: 2),
    );
    VisualizerStatusRequest().sendSignalToRust();
    try {
      final signal = await response;
      if (!mounted) return;
      setState(() => _builtinAvailable = signal.message.builtinAvailable);
    } on TimeoutException {
      // Without an answer only the folder is offered.
    }
  }

  Future<void> _chooseFolder() async {
    final dir = await getDirPath();
    if (dir == null || !mounted) return;

    try {
      await MacSecureManager().saveBookmark(dir);
    } catch (e) {
      error$('Failed to keep access to $dir: $e');
    }
    await $settingsManager.setValue(kVisualizerPresetDirKey, dir);
    await $settingsManager.setValue(kVisualizerSourceKey, 'folder');
    if (!mounted) return;

    // Choosing a folder also switches to it.
    setState(() {
      _presetDir = dir;
      _source = 'folder';
    });
  }

  Future<void> _selectSource(String? source) async {
    if (source == null) return;
    if (source == 'folder' && _presetDir == null) {
      await _chooseFolder();
      return;
    }
    await $settingsManager.setValue(kVisualizerSourceKey, source);
    if (!mounted) return;
    setState(() => _source = source);
  }

  @override
  Widget build(BuildContext context) {
    if (!Platform.isMacOS) {
      return const SizedBox.shrink();
    }

    final s = S.of(context);

    // Without a saved choice Rust uses the folder if one is set.
    final source = _source ?? (_presetDir != null ? 'folder' : 'builtin');

    return Column(
      children: [
        if (_builtinAvailable)
          SettingsBoxComboBox<String>(
            title: s.visualizerPresetSource,
            subtitle: s.visualizerPresetSourceSubtitle,
            value: source,
            items: [
              SettingsBoxComboBoxItem(
                value: 'builtin',
                title: s.visualizerSourceBuiltin,
              ),
              SettingsBoxComboBoxItem(
                value: 'folder',
                title: s.visualizerSourceFolder,
              ),
            ],
            onChanged: _selectSource,
          ),
        SettingsButton(
          icon: Symbols.folder_open,
          title: s.visualizerChooseFolder,
          subtitle: _presetDir ?? s.visualizerPresetFolderNotSet,
          onPressed: _chooseFolder,
        ),
      ],
    );
  }
}
