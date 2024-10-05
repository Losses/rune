import 'package:fluent_ui/fluent_ui.dart';

import '../../../widgets/track_list/track_list.dart';
import '../../../widgets/start_screen/start_screen.dart';
import '../../../screens/search/widgets/search_track_list.dart';
import '../../../messages/collection.pb.dart';

import '../utils/track_items_to_search_card.dart';
import '../utils/collection_items_to_search_card.dart';

class LargeScreenSearchTrackList extends StatelessWidget {
  final List<InternalMediaFile> tracks;
  final List<InternalCollection> artists;
  final List<InternalCollection> albums;
  final List<InternalCollection> playlists;

  const LargeScreenSearchTrackList({
    super.key,
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
            const SizedBox(height: 24),
            LayoutBuilder(
              builder: (context, constraints) {
                const double gapSize = 8;
                const double cellSize = 200;

                const ratio = 4 / 1;

                final int rows = (constraints.maxWidth / (cellSize + gapSize))
                    .floor()
                    .clamp(1, 0x7FFFFFFFFFFFFFFF);

                return Column(
                  children: [
                    Text("Artists", style: typography.bodyLarge),
                    SearchTrackList(
                      rows: rows,
                      ratio: ratio,
                      gapSize: gapSize,
                      cellSize: cellSize,
                      collectionType: CollectionType.Artist,
                      items: collectionItemsToSearchCard(
                        artists,
                        CollectionType.Artist,
                      ),
                    ),
                    Text("Albums", style: typography.bodyLarge),
                    SearchTrackList(
                      rows: rows,
                      ratio: ratio,
                      gapSize: gapSize,
                      cellSize: cellSize,
                      collectionType: CollectionType.Album,
                      items: collectionItemsToSearchCard(
                        albums,
                        CollectionType.Album,
                      ),
                    ),
                    Text("Playlists", style: typography.bodyLarge),
                    SearchTrackList(
                      rows: rows,
                      ratio: ratio,
                      gapSize: gapSize,
                      cellSize: cellSize,
                      collectionType: CollectionType.Playlist,
                      items: collectionItemsToSearchCard(
                        playlists,
                        CollectionType.Playlist,
                      ),
                    ),
                    Text("Tracks", style: typography.bodyLarge),
                    SearchTrackList(
                      rows: rows,
                      ratio: ratio,
                      gapSize: gapSize,
                      cellSize: cellSize,
                      collectionType: CollectionType.Track,
                      items: trackItemsToSearchCard(tracks),
                    ),
                  ],
                );
              },
            ),
          ],
        ),
      ),
    );
  }
}
