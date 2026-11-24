import 'package:fluent_ui/fluent_ui.dart';

import '../../../widgets/navigation_bar/page_content_frame.dart';
import '../../../widgets/start_screen/managed_start_screen_item.dart';
import '../../../bindings/bindings.dart';

import './search_card.dart';

const List<SearchCard> defaultList = [];

class LargeScreenSearchTrackListImplementation extends StatelessWidget {
  final int rows;
  final double ratio;
  final double gapSize;
  final double cellSize;
  final CollectionType collectionType;
  final List<SearchCard>? items;
  final int groupId;

  /// Null where the grid is the page's scroll view. Where it sits in one with
  /// other groups, a drag that starts on a card must reach that one, so the
  /// grid takes [NeverScrollableScrollPhysics].
  final ScrollPhysics? physics;

  const LargeScreenSearchTrackListImplementation({
    super.key,
    required this.rows,
    required this.ratio,
    required this.gapSize,
    required this.cellSize,
    required this.collectionType,
    required this.items,
    required this.groupId,
    this.physics,
  });

  @override
  Widget build(BuildContext context) {
    return GridView(
      shrinkWrap: true,
      physics: physics,
      keyboardDismissBehavior: ScrollViewKeyboardDismissBehavior.onDrag,
      gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
        crossAxisCount: rows,
        mainAxisSpacing: gapSize,
        crossAxisSpacing: gapSize,
        childAspectRatio: ratio,
      ),
      padding: getScrollContainerPadding(
        context,
        left: false,
        right: false,
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
