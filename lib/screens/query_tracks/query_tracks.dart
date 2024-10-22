import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/query_list.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../providers/responsive_providers.dart';

import 'query_tracks_list.dart';

class QueryTracksPage extends StatefulWidget {
  final QueryList queries;
  final int mode;
  final String? title;

  const QueryTracksPage({
    super.key,
    required this.queries,
    required this.mode,
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
      child: BreakpointBuilder(
        breakpoints: const [DeviceType.dock, DeviceType.band, DeviceType.tv],
        builder: (context, activeBreakpoint) {
          final isMini = activeBreakpoint == DeviceType.dock ||
              activeBreakpoint == DeviceType.band;
          return PageContentFrame(
            top: isMini,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                if (!isMini)
                  Padding(
                    padding: const EdgeInsets.fromLTRB(20, 54, 24, 12),
                    child: Transform.scale(
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
                    mode: widget.mode,
                  ),
                ),
              ],
            ),
          );
        },
      ),
    );
  }
}
