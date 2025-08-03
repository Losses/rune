import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:mesh_gradient/mesh_gradient.dart';
import 'package:animated_flip_counter/animated_flip_counter.dart';

import '../../utils/color_brightness.dart';
import '../../bindings/bindings.dart';
import '../../providers/library_path.dart';
import '../../providers/library_manager.dart';
import '../../utils/l10n.dart';

class ScanningPage extends StatefulWidget {
  const ScanningPage({super.key});

  @override
  State<ScanningPage> createState() => _ScanningPageState();
}

class _ScanningPageState extends State<ScanningPage>
    with SingleTickerProviderStateMixin {
  late final AnimatedMeshGradientController _animationController =
      AnimatedMeshGradientController();

  @override
  void initState() {
    super.initState();
    _animationController.start();
  }

  @override
  void dispose() {
    _animationController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final typography = theme.typography;

    final libraryPath = Provider.of<LibraryPathProvider>(context);
    final libraryManager = Provider.of<LibraryManagerProvider>(context);

    final progress =
        libraryManager.getScanTaskProgress(libraryPath.currentPath ?? "");

    final task = progress?.type ?? ScanTaskType.indexFiles;
    final count = progress?.progress ?? 0;

    return AnimatedMeshGradient(
      colors: [
        theme.accentColor.darker.darken(0.1),
        theme.accentColor.darker.darken(0.2),
        theme.accentColor.darker.darken(0.3),
        theme.accentColor.darker.darken(0.4),
      ],
      options: AnimatedMeshGradientOptions(),
      controller: _animationController,
      child: SizedBox.expand(
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          crossAxisAlignment: CrossAxisAlignment.center,
          children: [
            Text(
              S.of(context).thisMightTakeAFewMinutes,
              style: typography.title?.apply(
                fontWeightDelta: -20,
                color: Colors.white,
              ),
              textAlign: TextAlign.center,
            ),
            const SizedBox(height: 12),
            AnimatedFlipCounter(
              value: count,
              duration: const Duration(milliseconds: 400),
              curve: Curves.easeInOut,
              suffix: task == ScanTaskType.indexFiles
                  ? S.of(context).tracksFound
                  : S.of(context).albumCoversCollected,
              textStyle: typography.bodyLarge?.apply(
                color: Colors.white,
                fontWeightDelta: -50,
              ),
            ),
          ],
        ),
      ),
    );
  }
}
