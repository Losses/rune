import 'dart:ui';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:provider/provider.dart';
import 'package:reorderables/reorderables.dart';

import '../../../providers/visualizer_scenes.dart';
import '../../../utils/build_draggable_feedback.dart';
import '../../../utils/l10n.dart';
import '../../../utils/rune_log.dart';
import '../../../utils/visualizer_supported.dart';
import '../../../widgets/pointer_type.dart';
import '../../../widgets/settings/settings_block_title.dart';

/// The scenes of the visualizer: check the ones the menu offers, and drag them
/// into the order "next" goes through. The scene stays when the song changes.
/// The last checked scene cannot be unchecked.
/// Without a GPU for the scenes only the cover wall can be chosen; the other
/// scenes are greyed out.
class VisualizerScenesSetting extends StatelessWidget {
  const VisualizerScenesSetting({super.key});

  @override
  Widget build(BuildContext context) {
    if (!isVisualizerSupported) return const SizedBox.shrink();

    final s = S.of(context);
    final theme = FluentTheme.of(context);
    final provider = context.watch<VisualizerScenesProvider>();
    final scenes = provider.scenes;
    final enabledCount = scenes.where((scene) => scene.enabled).length;
    final noGpu = provider.gpu == false;

    return Padding(
      padding: const EdgeInsets.all(4),
      child: Expander(
        header: Padding(
          padding: const EdgeInsets.symmetric(vertical: 11),
          child: SettingsBlockTitle(
            title: s.visualizerScenes,
            subtitle: s.visualizerScenesSubtitle,
          ),
        ),
        content: SizedBox(
          width: double.maxFinite,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              if (noGpu)
                Padding(
                  padding: const EdgeInsets.only(bottom: 8),
                  child: InfoBar(
                    title: Text(s.visualizerNoGpu),
                    content: Text(provider.gpuProblem ?? ''),
                    severity: InfoBarSeverity.warning,
                    isLong: true,
                  ),
                ),
              PointerTypeBuilder(
                builder: (kind) {
                  return ReorderableColumn(
                    needsLongPressDraggable: kind == PointerDeviceKind.touch,
                    onReorder: provider.reorder,
                    buildDraggableFeedback: buildDraggableFeedback,
                    children: [
                      for (final scene in scenes)
                        Padding(
                          key: ValueKey(scene.path),
                          padding: const EdgeInsets.symmetric(vertical: 4),
                          child: Row(
                            children: [
                              Expanded(
                                child: Align(
                                  alignment: Alignment.centerLeft,
                                  child: Checkbox(
                                    content: Text.rich(
                                      TextSpan(
                                        children: [
                                          TextSpan(
                                            text: scene.isCoverWall
                                                ? s.coverWall
                                                : scene.name,
                                          ),
                                        ],
                                      ),
                                    ),
                                    checked: scene.enabled,
                                    onChanged:
                                        (noGpu && !scene.isCoverWall) ||
                                            (scene.enabled && enabledCount <= 1)
                                        ? null
                                        : (checked) {
                                            if (checked != null) {
                                              provider.setEnabled(
                                                scene.path,
                                                checked,
                                              );
                                            }
                                          },
                                  ),
                                ),
                              ),
                              if (scene.imported)
                                Tooltip(
                                  message: s.visualizerDeleteScene,
                                  child: IconButton(
                                    icon: const Icon(Symbols.delete),
                                    onPressed: () => _confirmDelete(
                                      context,
                                      provider,
                                      scene,
                                    ),
                                  ),
                                ),
                              Icon(
                                Symbols.drag_indicator,
                                color: theme.resources.textFillColorSecondary,
                              ),
                            ],
                          ),
                        ),
                    ],
                  );
                },
              ),
            ],
          ),
        ),
      ),
    );
  }

  // An imported scene is gone once deleted, until its file is imported again.
  Future<void> _confirmDelete(
    BuildContext context,
    VisualizerScenesProvider provider,
    VisualizerScene scene,
  ) async {
    final s = S.of(context);
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => ContentDialog(
        title: Text(s.visualizerDeleteScene),
        content: Text(s.visualizerDeleteSceneConfirm(scene.name)),
        actions: [
          FilledButton(
            child: Text(s.delete),
            onPressed: () => Navigator.pop(context, true),
          ),
          Button(
            child: Text(s.cancel),
            onPressed: () => Navigator.pop(context, false),
          ),
        ],
      ),
    );
    if (confirmed != true) return;
    final error = await provider.removeScene(scene);
    if (error != null) {
      error$('Failed to delete the scene ${scene.name}: $error');
    }
  }
}
