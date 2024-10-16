import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/widgets/navigation_bar/navigation_bar_placeholder.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import 'query_tracks_list.dart';

import '../../utils/query_list.dart';
import '../../widgets/playback_controller/controllor_placeholder.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';

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
      child: SmallerOrEqualTo(
          breakpoint: DeviceType.band,
          builder: (context, isBand) {
            return Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                if (!isBand)
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
                if (isBand) const NavigationBarPlaceholder(),
                Expanded(
                  child: QueryTrackListView(
                    layoutManager: _layoutManager,
                    queries: widget.queries,
                    mode: widget.mode,
                  ),
                ),
                const ControllerPlaceholder(),
              ],
            );
          }),
    );
  }
}
