import 'package:fluent_ui/fluent_ui.dart';

import '../../../screens/search/widgets/search_card.dart';
import '../../../screens/search/widgets/search_track_list.dart';
import '../../../messages/collection.pb.dart';

class LargeScreenSearchTrackList extends StatelessWidget {
  final CollectionType selectedItem;
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

                final List<SearchCard> itemGroup = items[selectedItem] ?? [];

                return SearchTrackList(
                  key: Key(selectedItem.toString()),
                  rows: rows,
                  ratio: ratio,
                  gapSize: gapSize,
                  cellSize: cellSize,
                  collectionType: selectedItem,
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
