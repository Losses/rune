import 'dart:io';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/query_list.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
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
      child: DeviceTypeBuilder(
        deviceType: const [DeviceType.dock, DeviceType.band, DeviceType.tv],
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
                    padding: Platform.isMacOS
                        // The left offset on macOS should be the same as the NavigationBar's parent title left offset
                        // But due to font and typography reasons(#166), we need to add 2px to visually align them.
                        ? const EdgeInsets.fromLTRB(26, 54, 24, 12)
                        : const EdgeInsets.fromLTRB(20, 54, 24, 12),
                    child: Transform.scale(
                      scale: 1.2,
                      alignment: Alignment.centerLeft,
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
                    topPadding: isMini,
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
