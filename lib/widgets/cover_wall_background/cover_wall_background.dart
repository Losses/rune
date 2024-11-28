import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/query_list.dart';
import '../../utils/settings_manager.dart';
import '../../utils/api/query_mix_tracks.dart';

import 'widgets/cover_wall_background_implementation.dart';

const coverCount = '40';

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
  Set<String> paths = {};

  @override
  void initState() {
    super.initState();
    loadCoverList();
  }

  loadCoverList() async {
    final String count =
        await SettingsManager().getValue<String?>(randomCoverWallCountKey) ??
            coverCount;

    final queryResult = await queryMixTracks(
      QueryList([
        ("lib::random", count.toString()),
        ("filter::with_cover_art", "true"),
      ]),
      0,
      int.parse(count),
    );

    if (!mounted) return;

    setState(() {
      paths = queryResult.map((x) => x.coverArtPath).toSet();
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
