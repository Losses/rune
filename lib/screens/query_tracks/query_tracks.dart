import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:player/utils/router_extra.dart';

import '../../screens/query_tracks/widgets/query_tracks.dart';

import '../../widgets/playback_controller.dart';

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
  @override
  Widget build(BuildContext context) {
    final FluentThemeData theme = FluentTheme.of(context);
    final Typography typography = theme.typography;
    final extra = GoRouterState.of(context).extra;

    return ScaffoldPage(
      header: HyperlinkButton(
        style: ButtonStyle(
          textStyle: WidgetStateProperty.all(typography.title),
        ),
        child: Text(extra is QueryTracksExtra ? extra.title : 'Tracks',
            style: TextStyle(color: theme.inactiveColor)),
        onPressed: () => {context.pop()},
      ),
      content: Column(children: [
        Expanded(
          child: QueryTrackListView(
            artistIds: widget.artistIds,
            albumIds: widget.albumIds,
            playlistIds: widget.playlistIds,
          ),
        ),
        const PlaybackPlaceholder(),
      ]),
    );
  }
}
