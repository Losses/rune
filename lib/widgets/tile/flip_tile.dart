import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_boring_avatars/flutter_boring_avatars.dart';
import 'package:rune/providers/responsive_providers.dart';

import 'tile.dart';
import 'fast_flip_cover_grid.dart';

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
    final colors = [
      theme.accentColor,
      theme.accentColor.light,
      theme.accentColor.lighter,
      theme.accentColor.lightest,
      theme.accentColor.dark,
      theme.accentColor.darker,
      theme.accentColor.darkest,
    ];

    return Tile(
      onPressed: onPressed,
      child: BreakpointBuilder(
        breakpoints: const [DeviceType.band, DeviceType.dock, DeviceType.tv],
        builder: (context, deviceType) {
          final isMini = deviceType == DeviceType.band || deviceType == DeviceType.dock;

          final coverArts = paths != null
              ? paths!.isNotEmpty
                  ? FastFlipCoverGrid(
                      size: 120,
                      name: name,
                      paths: paths!,
                      colors: colors,
                    )
                  : EmptyFlipCover(
                      name: name,
                      emptyTileType: emptyTileType,
                      colors: colors,
                    )
              : Container();

          if (isMini) return coverArts;

          return Stack(
            alignment: Alignment.bottomLeft,
            children: [
              coverArts,
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
          );
        },
      ),
    );
  }
}

class EmptyFlipCover extends StatelessWidget {
  const EmptyFlipCover({
    super.key,
    required this.name,
    required this.colors,
    required this.emptyTileType,
  });

  final String name;
  final BoringAvatarType emptyTileType;
  final List<Color> colors;

  @override
  Widget build(BuildContext context) {
    return BoringAvatar(
      name: name,
      palette: BoringAvatarPalette(colors),
      type: emptyTileType,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(0),
      ),
    );
  }
}
