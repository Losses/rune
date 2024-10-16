import 'package:fluent_ui/fluent_ui.dart';

import '../../../screens/search/widgets/search_card.dart';
import '../../../messages/collection.pb.dart';

import 'small_screen_search_track_list_implementation.dart';

class BandScreenSearchTrackList extends StatelessWidget {
  final Map<CollectionType, List<SearchCard>> items;

  const BandScreenSearchTrackList({
    super.key,
    required this.items,
  });

  @override
  Widget build(BuildContext context) {
    return Column(
      mainAxisAlignment: MainAxisAlignment.start,
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        LayoutBuilder(
          builder: (context, constraints) {
            return Column(
              mainAxisAlignment: MainAxisAlignment.start,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                SmallScreenSearchTrackListImplementation(
                  collectionType: CollectionType.Track,
                  items: items[CollectionType.Track],
                  groupId: 3,
                ),
              ],
            );
          },
        ),
      ],
    );
  }
}
