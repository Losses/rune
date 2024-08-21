import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_boring_avatars/flutter_boring_avatars.dart';

import 'flip_grid.dart';

class FlipTile extends StatelessWidget {
  final String name;
  final List<int> coverIds;
  final VoidCallback onPressed;
  final BoringAvatarsType emptyTileType;

  const FlipTile(
      {super.key,
      required this.name,
      required this.coverIds,
      required this.onPressed,
      this.emptyTileType = BoringAvatarsType.bauhaus});

  @override
  Widget build(BuildContext context) {
    return Button(
      style:
          const ButtonStyle(padding: WidgetStatePropertyAll(EdgeInsets.all(0))),
      onPressed: onPressed,
      child: ClipRRect(
        borderRadius: BorderRadius.circular(3),
        child: SizedBox(
          width: double.infinity,
          height: double.infinity,
          child: Stack(
            alignment: Alignment.bottomLeft,
            children: [
              FlipCoverGrid(
                  numbers: coverIds, id: name, emptyTileType: emptyTileType),
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
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
