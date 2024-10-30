import 'package:fluent_ui/fluent_ui.dart';

import '../../../screens/search/widgets/search_card.dart';
import '../../../messages/collection.pb.dart';

import 'small_screen_search_track_list_implementation.dart';

class BandScreenSearchTrackList extends StatelessWidget {
  final Map<CollectionType, List<SearchCard>> items;
  final Axis direction;

  const BandScreenSearchTrackList({
    super.key,
    required this.items,
    this.direction = Axis.vertical,
  });

  @override
  Widget build(BuildContext context) {
    return SmallScreenSearchTrackListImplementation(
      collectionType: CollectionType.Track,
      items: items[CollectionType.Track],
      groupId: 3,
      direction: direction,
    );
  }
}
