import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../screens/search/widgets/search_card.dart';
import '../../../bindings/bindings.dart';

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
                  if (items[CollectionType.artist]?.isNotEmpty ?? false)
                    SearchListSectionTitle(text: S.of(context).artists),
                  LargeScreenSearchTrackListImplementation(
                    rows: rows,
                    ratio: ratio,
                    gapSize: gapSize,
                    cellSize: cellSize,
                    collectionType: CollectionType.artist,
                    items: items[CollectionType.artist],
                    groupId: 0,
                  ),
                  if (items[CollectionType.album]?.isNotEmpty ?? false)
                    SearchListSectionTitle(text: S.of(context).albums),
                  LargeScreenSearchTrackListImplementation(
                    rows: rows,
                    ratio: ratio,
                    gapSize: gapSize,
                    cellSize: cellSize,
                    collectionType: CollectionType.album,
                    items: items[CollectionType.album],
                    groupId: 1,
                  ),
                  if (items[CollectionType.playlist]?.isNotEmpty ?? false)
                    SearchListSectionTitle(text: S.of(context).playlists),
                  LargeScreenSearchTrackListImplementation(
                    rows: rows,
                    ratio: ratio,
                    gapSize: gapSize,
                    cellSize: cellSize,
                    collectionType: CollectionType.playlist,
                    items: items[CollectionType.playlist],
                    groupId: 2,
                  ),
                  if (items[CollectionType.track]?.isNotEmpty ?? false)
                    SearchListSectionTitle(text: S.of(context).tracks),
                  LargeScreenSearchTrackListImplementation(
                    rows: rows,
                    ratio: ratio,
                    gapSize: gapSize,
                    cellSize: cellSize,
                    collectionType: CollectionType.track,
                    items: items[CollectionType.track],
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
