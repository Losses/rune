import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../providers/visualizer_scenes.dart';

/// Scenes draw on black, so while the cover wall shows one, [child] uses the
/// dark variant of the app theme even in light mode: dark overlays and light
/// text stay readable over the picture.
class VisualizerDarkTheme extends StatelessWidget {
  const VisualizerDarkTheme({
    super.key,
    required this.child,
    this.enabled = true,
  });

  final Widget child;

  /// Whether [child] is on the cover wall at all.
  final bool enabled;

  @override
  Widget build(BuildContext context) {
    final sceneShowing = context.select<VisualizerScenesProvider, bool>(
      (p) => p.sceneShowing,
    );
    final theme = FluentTheme.of(context);
    final dark = enabled && sceneShowing && !theme.brightness.isDark;

    // Always under a theme of its own, so that switching to the dark one does
    // not rebuild [child] from scratch (which would restart the visualizer).
    return FluentTheme(
      data: dark
          ? FluentThemeData(
              brightness: Brightness.dark,
              accentColor: theme.accentColor,
              visualDensity: theme.visualDensity,
              fontFamily: theme.typography.body?.fontFamily,
            )
          : theme,
      child: child,
    );
  }
}
