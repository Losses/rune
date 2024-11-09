import 'package:rinf/rinf.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:mesh_gradient/mesh_gradient.dart';

import '../../utils/color_brightness.dart';
import '../../messages/library_manage.pb.dart';
import '../../providers/library_path.dart';
import '../../providers/library_manager.dart';

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

    // Pass context to the onLibraryScanned function
    ScanAudioLibraryResponse.rustSignalStream.listen((rustSignal) {
      if (!mounted) return;
      onLibraryScanned(rustSignal, context);
    });
  }

  // Modify the function to accept BuildContext
  void onLibraryScanned(
      RustSignal<ScanAudioLibraryResponse> rustSignal, BuildContext context) {
    final libraryPath =
        Provider.of<LibraryPathProvider>(context, listen: false);
    final libraryManager =
        Provider.of<LibraryManagerProvider>(context, listen: false);

    final scanProgress =
        libraryManager.getScanTaskProgress(libraryPath.currentPath);
    final scanning = scanProgress?.status == TaskStatus.working;

    final currentPath = libraryPath.currentPath;
    if (scanning && currentPath != null) {
      libraryManager.analyseLibrary(currentPath);
    }
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

    final task = progress?.type ?? ScanTaskType.IndexFiles;
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
              "This might take a few minutes.",
              style: typography.title?.apply(
                fontWeightDelta: -20,
                color: Colors.white,
              ),
              textAlign: TextAlign.center,
            ),
            const SizedBox(height: 12),
            Text(
              count > 1
                  ? task == ScanTaskType.IndexFiles
                      ? '$count tracks found'
                      : '$count cover arts collected'
                  : "Sit back and relax",
              style: typography.bodyLarge?.apply(
                color: Colors.white,
                fontWeightDelta: -50,
              ),
            )
          ],
        ),
      ),
    );
  }
}
