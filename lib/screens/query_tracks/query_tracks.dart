import 'package:player/utils/query_list.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../screens/query_tracks/widgets/query_tracks.dart';

import '../../widgets/scaled.dart';
import '../../widgets/playback_controller/playback_placeholder.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';

class QueryTracksPage extends StatefulWidget {
  final QueryList queries;
  final String? title;

  const QueryTracksPage({
    super.key,
    required this.queries,
    required this.title,
  });

  @override
  State<QueryTracksPage> createState() => _QueryTracksPageState();
}

class _QueryTracksPageState extends State<QueryTracksPage> {
  final _layoutManager = StartScreenLayoutManager();

  @override
  Widget build(BuildContext context) {
    final FluentThemeData theme = FluentTheme.of(context);

    return ChangeNotifierProvider<StartScreenLayoutManager>.value(
      value: _layoutManager,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(20, 54, 24, 12),
            child: Scaled(
              scale: 1.2,
              child: Text(
                widget.title ?? 'Tracks',
                style: TextStyle(color: theme.inactiveColor),
              ),
            ),
          ),
          Expanded(
            child: QueryTrackListView(
              layoutManager: _layoutManager,
              queries: widget.queries,
            ),
          ),
          const PlaybackPlaceholder(),
        ],
      ),
    );
  }
}
