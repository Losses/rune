import 'package:player/providers/library_manager.dart';
import 'package:rinf/rinf.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:mesh_gradient/mesh_gradient.dart';

import '../../messages/library_manage.pb.dart';
import '../../providers/library_path.dart';

extension ColorBrightness on Color {
  Color darken([double amount = .1]) {
    assert(amount >= 0 && amount <= 1);

    final hsl = HSLColor.fromColor(this);
    final hslDark = hsl.withLightness((hsl.lightness - amount).clamp(0.0, 1.0));

    return hslDark.toColor();
  }

  Color lighten([double amount = .1]) {
    assert(amount >= 0 && amount <= 1);

    final hsl = HSLColor.fromColor(this);
    final hslLight =
        hsl.withLightness((hsl.lightness + amount).clamp(0.0, 1.0));

    return hslLight.toColor();
  }
}

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

    final currentPath = libraryPath.currentPath;
    libraryPath.finalizeScanning();
    if (libraryPath.scanning && currentPath != null) {
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

    return AnimatedMeshGradient(
      colors: [
        theme.accentColor.darker.darken(0.1),
        theme.accentColor.darker.darken(0.2),
        theme.accentColor.darker.darken(0.3),
        theme.accentColor.darker.darken(0.4),
      ],
      options: AnimatedMeshGradientOptions(),
      controller: _animationController,
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          Text(
            "This might take a few minutes.",
            style: typography.title?.apply(fontWeightDelta: -50),
          ),
          const SizedBox(height: 12),
          Text(
            "Sit back and relax",
            style: typography.bodyLarge,
          )
        ],
      ),
    );
  }
}