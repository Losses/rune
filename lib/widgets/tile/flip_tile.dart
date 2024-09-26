import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_boring_avatars/flutter_boring_avatars.dart';

import '../../utils/query_list.dart';

import 'flip_grid.dart';
import 'cover_art_manager.dart';

class FlipTile extends StatefulWidget {
  final String name;
  final QueryList queries;
  final VoidCallback onPressed;
  final BoringAvatarType emptyTileType;

  const FlipTile({
    super.key,
    required this.name,
    required this.queries,
    required this.onPressed,
    this.emptyTileType = BoringAvatarType.bauhaus,
  });

  @override
  FlipTileState createState() => FlipTileState();
}

class FlipTileState extends State<FlipTile> {
  late Future<List<int>> queryTask;

  @override
  void initState() {
    final coverArtManager = Provider.of<CoverArtManager>(context);
    queryTask = coverArtManager.queryCoverArts(widget.queries);

    super.initState();
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return Button(
      style: const ButtonStyle(
        padding: WidgetStatePropertyAll(
          EdgeInsets.all(0),
        ),
      ),
      onPressed: widget.onPressed,
      child: ClipRRect(
        borderRadius: BorderRadius.circular(3),
        child: SizedBox.expand(
          child: FutureBuilder<List<int>>(
            future: queryTask,
            builder: (context, snapshot) {
              if (snapshot.hasData) {
                return Stack(
                  alignment: Alignment.bottomLeft,
                  children: [
                    FlipCoverGrid(
                      coverArtIds: snapshot.data!,
                      id: widget.name,
                      emptyTileType: widget.emptyTileType,
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
                        widget.name,
                        textAlign: TextAlign.start,
                        style: theme.typography.body
                            ?.apply(color: theme.activeColor),
                      ),
                    ),
                  ],
                );
              }

              return Container();
            },
          ),
        ),
      ),
    );
  }
}
