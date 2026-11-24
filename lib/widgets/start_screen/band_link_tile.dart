import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/ax_pressure.dart';
import '../../widgets/tile/tile.dart';

import 'utils/get_tile_colors.dart';

class BandLinkTile extends StatelessWidget {
  final String title;
  final VoidCallback onPressed;
  final IconData icon;

  const BandLinkTile({
    super.key,
    required this.title,
    required this.onPressed,
    required this.icon,
  });

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    final List<Color> colors = getTileColors(theme);

    return AxPressure(
      child: Tile(
        onPressed: onPressed,
        child: Container(
          color: colors[title.hashCode % colors.length],
          child: Center(
            child: LayoutBuilder(
              builder: (context, constraints) {
                return Icon(
                  icon,
                  size: (constraints.maxWidth * 0.6).clamp(0, 40),
                  color: Colors.white,
                );
              },
            ),
          ),
        ),
      ),
    );
  }
}
