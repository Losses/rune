import 'package:file_selector/file_selector.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:go_router/go_router.dart';
import 'package:mesh_gradient/mesh_gradient.dart';
import 'package:provider/provider.dart';

import '../../utils/ax_shadow.dart';
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

class WelcomePage extends StatefulWidget {
  const WelcomePage({super.key});

  @override
  State<WelcomePage> createState() => _WelcomePageState();
}

class _WelcomePageState extends State<WelcomePage>
    with SingleTickerProviderStateMixin {
  bool scanning = false;
  late final AnimatedMeshGradientController _animationController =
      AnimatedMeshGradientController();

  @override
  void dispose() {
    _animationController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final typography = theme.typography;

    if (scanning) {
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

    return Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 400, maxHeight: 560),
        child: Container(
          decoration: BoxDecoration(
            color: theme.cardColor,
            borderRadius: BorderRadius.circular(3),
            boxShadow: axShadow(20),
          ),
          child: Padding(
            padding: const EdgeInsets.all(12),
            child: Column(
              mainAxisAlignment: MainAxisAlignment.start,
              children: [
                Expanded(
                  child: Column(
                    mainAxisAlignment: MainAxisAlignment.center,
                    crossAxisAlignment: CrossAxisAlignment.center,
                    children: [
                      SvgPicture.asset(
                        'assets/mono_color_logo.svg',
                        colorFilter: ColorFilter.mode(
                            theme.activeColor, BlendMode.srcIn),
                      ),
                      const SizedBox(height: 20),
                      Column(
                        children: [
                          const Padding(
                            padding: EdgeInsets.symmetric(horizontal: 24),
                            child: Text(
                              'Select your audio library directory, and we will scan and analysis all tracks within it.',
                              textAlign: TextAlign.center,
                              style: TextStyle(height: 1.4),
                            ),
                          ),
                          const SizedBox(height: 56),
                          FilledButton(
                            child: const Text("Select Directory"),
                            onPressed: () async {
                              final result = await getDirectoryPath();

                              if (result == null) {
                                return;
                              }

                              setState(() {
                                scanning = true;
                              });

                              _animationController.isAnimating.value
                                  ? _animationController.stop()
                                  : _animationController.start();

                              if (!context.mounted) return;

                              await scanLibrary(context, result);

                              if (!context.mounted) return;
                              context.go("/library");
                            },
                          ),
                        ],
                      ),
                    ],
                  ),
                ),
                Text(
                  '© 2024 Rune Player Developers. Licensed under MPL 2.0.',
                  style: theme.typography.caption
                      ?.apply(color: theme.activeColor.withAlpha(80)),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  Future<void> scanLibrary(BuildContext context, String path) async {
    final library = Provider.of<LibraryPathProvider>(context, listen: false);

    await library.setLibraryPath(path, true);
    ScanAudioLibraryRequest(path: path).sendSignalToRust();

    while (true) {
      final rustSignal = await ScanAudioLibraryResponse.rustSignalStream.first;

      if (rustSignal.message.path == path) {
        library.finalizeScanning();
        return;
      }
    }
  }
}
