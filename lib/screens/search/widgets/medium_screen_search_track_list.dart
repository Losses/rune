import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../screens/search/widgets/search_card.dart';
import '../../../messages/collection.pb.dart';

import 'large_screen_search_track_list_implementation.dart';

class MediumScreenSearchTrackList extends StatelessWidget {
  final Map<CollectionType, List<SearchCard>> items;

  const MediumScreenSearchTrackList({
    super.key,
    required this.items,
  });

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 32),
      child: Column(
        mainAxisAlignment: MainAxisAlignment.start,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          LayoutBuilder(
            builder: (context, constraints) {
              const double gapSize = 8;
              const double cellSize = 200;

              const ratio = 4 / 1;

              final int rows = max(
                (constraints.maxWidth / (cellSize + gapSize)).floor(),
                1,
              );

              return Column(
                mainAxisAlignment: MainAxisAlignment.start,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  if (items[CollectionType.Artist]?.isNotEmpty ?? false)
                    SearchListSectionTitle(text: S.of(context).artists),
                  LargeScreenSearchTrackListImplementation(
                    rows: rows,
                    ratio: ratio,
                    gapSize: gapSize,
                    cellSize: cellSize,
                    collectionType: CollectionType.Artist,
                    items: items[CollectionType.Artist],
                    groupId: 0,
                  ),
                  if (items[CollectionType.Album]?.isNotEmpty ?? false)
                    SearchListSectionTitle(text: S.of(context).albums),
                  LargeScreenSearchTrackListImplementation(
                    rows: rows,
                    ratio: ratio,
                    gapSize: gapSize,
                    cellSize: cellSize,
                    collectionType: CollectionType.Album,
                    items: items[CollectionType.Album],
                    groupId: 1,
                  ),
                  if (items[CollectionType.Playlist]?.isNotEmpty ?? false)
                    SearchListSectionTitle(text: S.of(context).playlists),
                  LargeScreenSearchTrackListImplementation(
                    rows: rows,
                    ratio: ratio,
                    gapSize: gapSize,
                    cellSize: cellSize,
                    collectionType: CollectionType.Playlist,
                    items: items[CollectionType.Playlist],
                    groupId: 2,
                  ),
                  if (items[CollectionType.Track]?.isNotEmpty ?? false)
                    SearchListSectionTitle(text: S.of(context).tracks),
                  LargeScreenSearchTrackListImplementation(
                    rows: rows,
                    ratio: ratio,
                    gapSize: gapSize,
                    cellSize: cellSize,
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
