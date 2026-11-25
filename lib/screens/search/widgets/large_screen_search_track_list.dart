import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';

import '../../../screens/search/widgets/search_card.dart';
import '../../../bindings/bindings.dart';

import 'large_screen_search_track_list_implementation.dart';

class LargeScreenSearchTrackList extends StatelessWidget {
  final (CollectionType, String Function(BuildContext)) selectedItem;
  final Map<CollectionType, List<SearchCard>> items;

  const LargeScreenSearchTrackList({
    super.key,
    required this.selectedItem,
    required this.items,
  });

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final typography = theme.typography;

    return Padding(
      padding: const EdgeInsets.only(top: 24, left: 32, right: 32),
      child: Column(
        mainAxisAlignment: MainAxisAlignment.start,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const SizedBox(height: 12),
          Text(selectedItem.$2(context), style: typography.title),
          const SizedBox(height: 24),
          Expanded(
            child: LayoutBuilder(
              builder: (context, constraints) {
                const double gapSize = 8;
                const double cellSize = 200;

                const ratio = 4 / 1;

                final int rows = max(
                    (constraints.maxWidth / (cellSize + gapSize)).floor(), 1);

                final List<SearchCard> itemGroup = items[selectedItem.$1] ?? [];

                return LargeScreenSearchTrackListImplementation(
                  key: Key(selectedItem.toString()),
                  rows: rows,
                  ratio: ratio,
                  gapSize: gapSize,
                  cellSize: cellSize,
                  collectionType: selectedItem.$1,
                  items: itemGroup,
                  groupId: 0,
                );
              },
            ),
          ),
        ],
      ),
    );
  }
}
