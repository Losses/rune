import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../screens/search/widgets/search_card.dart';
import '../../../bindings/bindings.dart';

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
                  if (items[CollectionType.artist]?.isNotEmpty ?? false)
                    SearchListSectionTitle(text: S.of(context).artists),
                  SmallScreenSearchTrackListImplementation(
                    collectionType: CollectionType.artist,
                    items: items[CollectionType.artist],
                    groupId: 0,
                    direction: Axis.vertical,
                  ),
                  if (items[CollectionType.album]?.isNotEmpty ?? false)
                    SearchListSectionTitle(text: S.of(context).albums),
                  SmallScreenSearchTrackListImplementation(
                    collectionType: CollectionType.album,
                    items: items[CollectionType.album],
                    groupId: 1,
                    direction: Axis.vertical,
                  ),
                  if (items[CollectionType.playlist]?.isNotEmpty ?? false)
                    SearchListSectionTitle(text: S.of(context).playlists),
                  SmallScreenSearchTrackListImplementation(
                    collectionType: CollectionType.playlist,
                    items: items[CollectionType.playlist],
                    groupId: 2,
                    direction: Axis.vertical,
                  ),
                  if (items[CollectionType.track]?.isNotEmpty ?? false)
                    SearchListSectionTitle(text: S.of(context).tracks),
                  SmallScreenSearchTrackListImplementation(
                    collectionType: CollectionType.track,
                    items: items[CollectionType.track],
                    groupId: 3,
                    direction: Axis.vertical,
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
