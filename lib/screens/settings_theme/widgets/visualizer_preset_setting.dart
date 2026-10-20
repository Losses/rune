import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';
import '../../../utils/rune_log.dart';
import '../../../utils/get_dir_path.dart';
import '../../../utils/visualizer_supported.dart';
import '../../../utils/file_storage/mac_secure_manager.dart';
import '../../../widgets/settings/settings_button.dart';
import '../../../constants/configurations.dart';
import '../../../constants/settings_manager.dart';
import '../../../bindings/bindings.dart';

/// The preset folder of the visualizer. It is saved here and also sent to
/// Rust, which is shared by all visualizers of the app, so a running
/// visualizer follows at once. Without a folder the visualizer shows its
/// default preset.
class VisualizerPresetSetting extends StatefulWidget {
  const VisualizerPresetSetting({super.key});

  @override
  State<VisualizerPresetSetting> createState() =>
      _VisualizerPresetSettingState();
}

class _VisualizerPresetSettingState extends State<VisualizerPresetSetting> {
  String? _presetDir;

  @override
  void initState() {
    super.initState();
    if (isVisualizerSupported) _load();
  }

  Future<void> _load() async {
    final dir = await $settingsManager.getValue<String>(
      kVisualizerPresetDirKey,
    );
    if (!mounted) return;
    setState(() => _presetDir = dir != null && dir.isNotEmpty ? dir : null);
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
    if (!mounted) return;
    setState(() => _presetDir = dir);

    // A visualizer that is open switches to the new folder now; one that is
    // not open reads the setting when it starts.
    SetVisualizerPresetDirRequest(path: dir).sendSignalToRust();
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

  Future<void> _stopUsingFolder() async {
    await $settingsManager.removeValue(kVisualizerPresetDirKey);
    if (!mounted) return;
    setState(() => _presetDir = null);

    // A visualizer that is open goes back to its default preset.
    ClearVisualizerPresetDirRequest().sendSignalToRust();
  }

  @override
  Widget build(BuildContext context) {
    if (!isVisualizerSupported) {
      return const SizedBox.shrink();
    }

    final s = S.of(context);
    final dir = _presetDir;

    return Column(
      children: [
        SettingsButton(
          icon: Symbols.folder_open,
          title: s.visualizerChooseFolder,
          subtitle: dir ?? s.visualizerPresetFolderNotSet,
          onPressed: _chooseFolder,
        ),
        if (dir != null)
          SettingsButton(
            icon: Symbols.folder_off,
            title: s.visualizerStopUsingFolder,
            subtitle: s.visualizerStopUsingFolderSubtitle,
            onPressed: _stopUsingFolder,
          ),
      ],
    );
  }
}
