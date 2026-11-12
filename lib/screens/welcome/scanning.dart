import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:mesh_gradient/mesh_gradient.dart';
import 'package:animated_flip_counter/animated_flip_counter.dart';

import '../../utils/color_brightness.dart';
import '../../providers/library_open_state.dart';
import '../../providers/library_manager.dart';
import '../../bindings/bindings.dart';
import '../../utils/l10n.dart';
import '../../utils/api/request_failure.dart';
import '../../utils/router/navigation.dart';
import '../../widgets/status_bar_style.dart';

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

    final libraryPath = Provider.of<LibraryOpenStateProvider>(context);
    final libraryManager = Provider.of<LibraryManagerProvider>(context);

    final recordId = libraryPath.openRecord?.recordId;
    final progress = libraryManager.getScanTaskProgress(recordId);

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
      // The page's background is dark in both themes.
      child: StatusBarStyle(
        brightness: Brightness.dark,
        child: SizedBox.expand(
          child: SafeArea(
            bottom: false,
            child: Column(
              mainAxisAlignment: MainAxisAlignment.center,
              crossAxisAlignment: CrossAxisAlignment.center,
              children: [
                if (progress?.status == TaskStatus.failed ||
                    progress?.status == TaskStatus.cancelled) ...[
                  Text(
                    progress?.failure != null
                        ? requestFailureMessage(
                            S.of(context),
                            progress!.failure!,
                          )
                        : S.of(context).error(progress?.error ?? ''),
                  ),
                  const SizedBox(height: 12),
                  Row(
                    mainAxisAlignment: MainAxisAlignment.center,
                    children: [
                      FilledButton(
                        onPressed: () => libraryManager.scanLibrary(
                          recordId ?? '',
                          isInitializeTask: true,
                        ),
                        child: Text(S.of(context).scan),
                      ),
                      const SizedBox(width: 12),
                      Button(
                        onPressed: () => $hasLibraryNavigator
                            ? $replace('/library')
                            : $$replace('/'),
                        child: Text(S.of(context).cancel),
                      ),
                    ],
                  ),
                ] else ...[
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
                    suffix: progress?.phase == ScanTaskPhase.indexFiles
                        ? S.of(context).tracksFound
                        : S.of(context).albumCoversCollected,
                    textStyle: typography.bodyLarge?.apply(
                      color: Colors.white,
                      fontWeightDelta: -50,
                    ),
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}
