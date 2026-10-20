import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../providers/cover_wall_display.dart';

/// MilkDrop presets draw on black, so while the cover wall shows the
/// visualizer, [child] uses the dark variant of the app theme even in light
/// mode: dark overlays and light text stay readable over the picture.
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
    final showVisualizer = context.select<CoverWallDisplayProvider, bool>(
      (p) => p.showVisualizer,
    );
    final theme = FluentTheme.of(context);
    if (!enabled || !showVisualizer || theme.brightness.isDark) return child;

    return FluentTheme(
      data: FluentThemeData(
        brightness: Brightness.dark,
        accentColor: theme.accentColor,
        visualDensity: theme.visualDensity,
        fontFamily: theme.typography.body?.fontFamily,
      ),
      child: child,
    );
  }
}
