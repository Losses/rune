import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_boring_avatars/flutter_boring_avatars.dart';

import 'flip_grid.dart';

class FlipTile extends StatelessWidget {
  final String name;
  final List<String>? paths;
  final VoidCallback onPressed;
  final BoringAvatarType emptyTileType;

  const FlipTile({
    super.key,
    required this.name,
    required this.paths,
    required this.onPressed,
    this.emptyTileType = BoringAvatarType.bauhaus,
  });

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return Button(
      style: const ButtonStyle(
        padding: WidgetStatePropertyAll(
          EdgeInsets.all(0),
        ),
      ),
      onPressed: onPressed,
      child: ClipRRect(
        borderRadius: BorderRadius.circular(3),
        child: SizedBox.expand(
          child: Stack(
            alignment: Alignment.bottomLeft,
            children: [
              if (paths != null)
                FlipCoverGrid(
                  paths: paths!,
                  id: name,
                  emptyTileType: emptyTileType,
                ),
              Container(
                decoration: BoxDecoration(
                  gradient: LinearGradient(
                    begin: const Alignment(0.0, -1.0),
                    end: const Alignment(0.0, 1.0),
                    colors: [
                      Colors.black.withAlpha(0),
                      Colors.black.withAlpha(160),
                    ],
                  ),
                ),
                height: 80,
              ),
              Padding(
                padding: const EdgeInsets.all(6),
                child: Text(
                  name,
                  textAlign: TextAlign.start,
                  style: theme.typography.body?.apply(color: theme.activeColor),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
