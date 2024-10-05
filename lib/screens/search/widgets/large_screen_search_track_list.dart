import 'package:fluent_ui/fluent_ui.dart';

import '../../../widgets/track_list/track_list.dart';
import '../../../widgets/start_screen/start_screen.dart';
import '../../../screens/search/widgets/search_card.dart';
import '../../../screens/search/widgets/search_track_list.dart';
import '../../../screens/search/utils/track_items_to_search_card.dart';
import '../../../screens/search/utils/collection_items_to_search_card.dart';
import '../../../messages/collection.pb.dart';

class LargeScreenSearchTrackList extends StatelessWidget {
  final CollectionType selectedItem;
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
            Text('${selectedItem.toString()}s', style: typography.title),
            const SizedBox(height: 24),
            Expanded(
              child: LayoutBuilder(
                builder: (context, constraints) {
                  const double gapSize = 8;
                  const double cellSize = 200;

                  const ratio = 4 / 1;

                  final int rows = (constraints.maxWidth / (cellSize + gapSize))
                      .floor()
                      .clamp(1, 0x7FFFFFFFFFFFFFFF);

                  final List<SearchCard> items;

                  switch (selectedItem) {
                    case CollectionType.Artist:
                      items =
                          collectionItemsToSearchCard(artists, selectedItem);
                      break;
                    case CollectionType.Album:
                      items = collectionItemsToSearchCard(albums, selectedItem);
                      break;
                    case CollectionType.Playlist:
                      items =
                          collectionItemsToSearchCard(playlists, selectedItem);
                    case CollectionType.Track:
                      items = trackItemsToSearchCard(tracks);
                      break;
                    default:
                      items = [];
                  }

                  return SearchTrackList(
                    key: Key(selectedItem.toString()),
                    rows: rows,
                    ratio: ratio,
                    gapSize: gapSize,
                    cellSize: cellSize,
                    collectionType: selectedItem,
                    items: items,
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
