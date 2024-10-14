import 'package:go_router/go_router.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/start_screen/band_link_tile.dart';

class BandLinkTileList extends StatelessWidget {
  const BandLinkTileList({
    super.key,
    required this.links,
  });

  final List<(String, String, IconData, bool)> links;

  @override
  Widget build(BuildContext context) {
    return SingleChildScrollView(
      child: Column(
        children: links
            .map(
              (item) => Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: 2,
                  vertical: 1,
                ),
                child: AspectRatio(
                  aspectRatio: 1,
                  child: BandLinkTile(
                    title: item.$1,
                    onPressed: () {
                      context.push(item.$2);
                    },
                    icon: item.$3,
                  ),
                ),
              ),
            )
            .toList(),
      ),
    );
  }
}
