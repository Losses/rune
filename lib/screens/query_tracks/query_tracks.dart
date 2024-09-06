import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';

import '../../utils/router_extra.dart';

import '../../screens/query_tracks/widgets/query_tracks.dart';

import '../../widgets/scaled.dart';
import '../../widgets/playback_controller.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';

class QueryTracksPage extends StatefulWidget {
  final List<int> artistIds;
  final List<int> albumIds;
  final List<int> playlistIds;

  const QueryTracksPage(
      {super.key,
      this.artistIds = const [],
      this.albumIds = const [],
      this.playlistIds = const []});

  @override
  State<QueryTracksPage> createState() => _QueryTracksPageState();
}

class _QueryTracksPageState extends State<QueryTracksPage> {
  final _layoutManager = StartScreenLayoutManager();

  @override
  Widget build(BuildContext context) {
    final FluentThemeData theme = FluentTheme.of(context);
    final extra = GoRouterState.of(context).extra;

    return ChangeNotifierProvider<StartScreenLayoutManager>.value(
        value: _layoutManager,
        child: ScaffoldPage(
          content:
              Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(20, 32, 24, 12),
              child: Scaled(
                  scale: 1.2,
                  child: Text(
                      extra is QueryTracksExtra ? extra.title : 'Tracks',
                      style: TextStyle(color: theme.inactiveColor))),
            ),
            Expanded(
              child: QueryTrackListView(
                layoutManager: _layoutManager,
                artistIds: widget.artistIds,
                albumIds: widget.albumIds,
                playlistIds: widget.playlistIds,
              ),
            ),
            const PlaybackPlaceholder(),
          ]),
        ));
  }
}
