import 'package:fluent_ui/fluent_ui.dart';

import '../../../widgets/track_list/track_list.dart';
import '../../../widgets/start_screen/start_screen.dart';
import '../../../widgets/start_screen/providers/managed_start_screen_item.dart';
import '../../../messages/collection.pb.dart';

import './track_search_item.dart';
import './collection_search_item.dart';

class LargeScreenSearchTrackList extends StatelessWidget {
  final String selectedItem;
  final List<InternalMediaFile> tracks;
  final List<InternalCollection> artists;
  final List<InternalCollection> albums;
  final List<InternalCollection> playlists;

  const LargeScreenSearchTrackList({
    super.key,
    required this.selectedItem,
    required this.tracks,
    required this.artists,
    required this.albums,
    required this.playlists,
  });

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final typography = theme.typography;

    return Expanded(
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 24, horizontal: 32),
        child: Column(
          mainAxisAlignment: MainAxisAlignment.start,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const SizedBox(height: 12),
            Text(selectedItem, style: typography.title),
            const SizedBox(height: 24),
            Expanded(
              child: LayoutBuilder(
                builder: (context, constraints) {
                  const double gapSize = 8;
                  const double cellSize = 200;

                  const ratio = 4 / 1;

                  final int rows =
                      (constraints.maxWidth / (cellSize + gapSize)).floor();

                  final trackIds = tracks.map((x) => x.id).toList();

                  return GridView(
                    key: Key(selectedItem),
                    gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
                      crossAxisCount: rows,
                      mainAxisSpacing: gapSize,
                      crossAxisSpacing: gapSize,
                      childAspectRatio: ratio,
                    ),
                    children: [
                      if (selectedItem == "Artists")
                        ...artists.map(
                          (a) => CollectionSearchItem(
                            item: a,
                            collectionType: CollectionType.Artist,
                          ),
                        ),
                      if (selectedItem == "Albums")
                        ...albums.map(
                          (a) => CollectionSearchItem(
                            item: a,
                            collectionType: CollectionType.Album,
                          ),
                        ),
                      if (selectedItem == "Playlists")
                        ...playlists.map(
                          (a) => CollectionSearchItem(
                            item: a,
                            collectionType: CollectionType.Playlist,
                          ),
                        ),
                      if (selectedItem == "Tracks")
                        ...tracks.map((a) => TrackSearchItem(
                              index: 0,
                              item: a,
                              fallbackFileIds: trackIds,
                            )),
                    ].asMap().entries.map((x) {
                      final index = x.key;
                      final int row = index % rows;
                      final int column = index ~/ rows;

                      return ManagedStartScreenItem(
                        key: Key('$selectedItem-$row:$column'),
                        prefix: selectedItem,
                        groupId: 0,
                        row: row,
                        column: column,
                        width: cellSize / ratio,
                        height: cellSize,
                        child: x.value,
                      );
                    }).toList(),
                  );
                },
              ),
            ),
          ],
        ),
      ),
    );
  }
}
