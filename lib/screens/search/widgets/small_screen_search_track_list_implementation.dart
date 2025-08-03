import 'package:fluent_ui/fluent_ui.dart';

import '../../../widgets/turntile/managed_turntile_screen_item.dart';
import '../../../bindings/bindings.dart';

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
    return Flex(
      direction: direction,
      children: (items ?? defaultList).asMap().entries.map(
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
      ).toList(),
    );
  }
}
