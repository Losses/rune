import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/query_list.dart';
import '../../utils/settings_manager.dart';
import '../../utils/api/query_mix_tracks.dart';

import 'widgets/cover_wall_background_implementation.dart';

const coverCount = 40;

const randomCoverWallCountKey = 'random_cover_wall_count';

class CoverWallBackground extends StatefulWidget {
  final int seed;
  final int gap;

  const CoverWallBackground({
    super.key,
    required this.seed,
    required this.gap,
  });

  @override
  State<CoverWallBackground> createState() => _CoverWallBackgroundState();
}

class _CoverWallBackgroundState extends State<CoverWallBackground> {
  final Set<String> paths = {};

  @override
  void initState() {
    super.initState();
    loadCoverList();
  }

  loadCoverList() async {
    final queryResult = await queryMixTracks(
      QueryList([
        (
          "lib::random",
          (await SettingsManager().getValue<int?>(randomCoverWallCountKey) ??
                  coverCount)
              .toString()
        ),
        ("filter::with_cover_art", "true"),
      ]),
    );

    if (!mounted) return;

    setState(() {
      for (final file in queryResult) {
        paths.add(file.coverArtPath);
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    if (paths.isEmpty) {
      return Container();
    }

    return LayoutBuilder(
      builder: (context, constraints) => CoverWallBackgroundImplementation(
        seed: widget.seed,
        gap: widget.gap,
        paths: paths.toList(),
        constraints: constraints,
      ),
    );
  }
}
