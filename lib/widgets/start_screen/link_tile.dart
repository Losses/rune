import 'package:go_router/go_router.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/tile/tile.dart';
import '../../widgets/ax_pressure.dart';

import 'utils/get_tile_colors.dart';

class LinkTile extends StatelessWidget {
  final String title;
  final String path;
  final IconData icon;

  const LinkTile({
    super.key,
    required this.title,
    required this.path,
    required this.icon,
  });

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    final List<Color> colors = getTileColors(theme);

    return AxPressure(
      child: Tile(
        onPressed: () {
          context.push(path);
        },
        child: Stack(
          alignment: Alignment.bottomLeft,
          children: [
            Container(
              color: colors[path.hashCode % colors.length],
              child: Center(
                  child: Icon(
                icon,
                size: 40,
                color: Colors.white,
              )),
            ),
            Padding(
              padding: const EdgeInsets.all(6),
              child: Text(
                title,
                textAlign: TextAlign.start,
                style: theme.typography.body?.apply(color: theme.activeColor),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
