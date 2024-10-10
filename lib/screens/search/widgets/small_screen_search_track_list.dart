import 'package:fluent_ui/fluent_ui.dart';

import '../../../screens/search/widgets/search_card.dart';
import '../../../messages/collection.pb.dart';

import 'small_screen_search_track_list_implementation.dart';

class SmallScreenSearchTrackList extends StatelessWidget {
  final Map<CollectionType, List<SearchCard>> items;

  const SmallScreenSearchTrackList({
    super.key,
    required this.items,
  });

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 16),
      child: Column(
        mainAxisAlignment: MainAxisAlignment.start,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          LayoutBuilder(
            builder: (context, constraints) {
              return Column(
                mainAxisAlignment: MainAxisAlignment.start,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  if (items[CollectionType.Artist]?.isNotEmpty ?? false)
                    const SearchListSectionTitle(text: "Artists"),
                  SmallScreenSearchTrackListImplementation(
                    collectionType: CollectionType.Artist,
                    items: items[CollectionType.Artist],
                    groupId: 0,
                  ),
                  if (items[CollectionType.Album]?.isNotEmpty ?? false)
                    const SearchListSectionTitle(text: "Albums"),
                  SmallScreenSearchTrackListImplementation(
                    collectionType: CollectionType.Album,
                    items: items[CollectionType.Album],
                    groupId: 1,
                  ),
                  if (items[CollectionType.Playlist]?.isNotEmpty ?? false)
                    const SearchListSectionTitle(text: "Playlists"),
                  SmallScreenSearchTrackListImplementation(
                    collectionType: CollectionType.Playlist,
                    items: items[CollectionType.Playlist],
                    groupId: 2,
                  ),
                  if (items[CollectionType.Track]?.isNotEmpty ?? false)
                    const SearchListSectionTitle(text: "Tracks"),
                  SmallScreenSearchTrackListImplementation(
                    collectionType: CollectionType.Track,
                    items: items[CollectionType.Track],
                    groupId: 3,
                  ),
                ],
              );
            },
          ),
        ],
      ),
    );
  }
}

class SearchListSectionTitle extends StatelessWidget {
  final String text;

  const SearchListSectionTitle({
    super.key,
    required this.text,
  });

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final typography = theme.typography;

    return Padding(
      padding: const EdgeInsets.fromLTRB(0, 16, 0, 4),
      child: Text(text, style: typography.bodyLarge),
    );
  }
}
