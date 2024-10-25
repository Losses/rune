import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/widgets/turntile/managed_turntile_screen_item.dart';

import '../../../messages/collection.pb.dart';

import './search_card.dart';

const List<SearchCard> defaultList = [];

class SmallScreenSearchTrackListImplementation extends StatelessWidget {
  final CollectionType collectionType;
  final List<SearchCard>? items;
  final int groupId;

  final Axis direction;

  const SmallScreenSearchTrackListImplementation({
    super.key,
    required this.collectionType,
    required this.items,
    required this.groupId,
    required this.direction,
  });

  @override
  Widget build(BuildContext context) {
    final list = (items ?? defaultList).asMap().entries.map(
      (x) {
        final index = x.key;
        final key = '${collectionType.toString()}-$index';

        return ManagedTurntileScreenItem(
          key: Key(key),
          prefix: collectionType.toString(),
          groupId: groupId,
          row: index,
          column: 1,
          child: x.value,
        );
      },
    ).toList();

    if (direction == Axis.vertical) {
      return Column(
        children: list,
      );
    }

    return Row(
      children: list,
    );
  }
}
