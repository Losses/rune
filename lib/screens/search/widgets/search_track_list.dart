import 'package:fluent_ui/fluent_ui.dart';

import '../../../widgets/start_screen/managed_start_screen_item.dart';
import '../../../messages/collection.pb.dart';

import './search_card.dart';

const List<SearchCard> defaultList = [];

class SearchTrackList extends StatelessWidget {
  final int rows;
  final double ratio;
  final double gapSize;
  final double cellSize;
  final CollectionType collectionType;
  final List<SearchCard>? items;
  final int groupId; 

  const SearchTrackList({
    super.key,
    required this.rows,
    required this.ratio,
    required this.gapSize,
    required this.cellSize,
    required this.collectionType,
    required this.items,
    required this.groupId,
  });

  @override
  Widget build(BuildContext context) {
    return GridView(
      physics: const NeverScrollableScrollPhysics(),
      shrinkWrap: true,
      gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
        crossAxisCount: rows,
        mainAxisSpacing: gapSize,
        crossAxisSpacing: gapSize,
        childAspectRatio: ratio,
      ),
      children: (items ?? defaultList).asMap().entries.map(
        (x) {
          final index = x.key;
          final int row = index % rows;
          final int column = index ~/ rows;
          final key = '${collectionType.toString()}-$row:$column';

          return ManagedStartScreenItem(
            key: Key(key),
            prefix: collectionType.toString(),
            groupId: groupId,
            row: row,
            column: column,
            width: cellSize / ratio,
            height: cellSize,
            child: x.value,
          );
        },
      ).toList(),
    );
  }
}
