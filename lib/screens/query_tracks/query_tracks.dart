import 'dart:io';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/kept_clear.dart';
import '../../utils/api/query_mix_tracks.dart';
import '../../utils/query_list.dart';
import '../../widgets/navigation_bar/flip_text.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../providers/responsive_providers.dart';
import '../../bindings/bindings.dart';

import 'query_tracks_list.dart';

/// The title of a page of tracks, at the top below the status bar.
class QueryTracksTitle extends StatelessWidget {
  final String title;

  /// The key the title flips with to and from the title of the header, for a
  /// page the header names above the pages it opens. Null for a plain title.
  final String? flipKey;

  const QueryTracksTitle({super.key, required this.title, this.flipKey});

  /// The scale the title is drawn at, and the size [FlipText] draws its text
  /// in before it scales it.
  static const _scale = 1.2;
  static const _flipTextSize = 17.0;

  @override
  Widget build(BuildContext context) {
    final FluentThemeData theme = FluentTheme.of(context);
    final keptClear = context.keptClear;
    // The size of the text the plain title is drawn in.
    final bodySize = DefaultTextStyle.of(context).style.fontSize ?? 14;

    return Padding(
      padding: Platform.isMacOS
          // The left offset on macOS should be the same as the NavigationBar's parent title left offset
          // But due to font and typography reasons(#166), we need to add 2px to visually align them.
          ? const EdgeInsets.fromLTRB(26, 54, 24, 12)
          : EdgeInsets.fromLTRB(
              20 + keptClear.left,
              54,
              24 + keptClear.right,
              12,
            ),
      child: flipKey == null
          ? Transform.scale(
              scale: _scale,
              alignment: Alignment.centerLeft,
              child: Text(title, style: TextStyle(color: theme.inactiveColor)),
            )
          // The same text as a text that flips, which is one of 17 scaled
          // from its top left: scaled to the plain title's size at 1.2, in a
          // box of the plain title's size, and moved up by what the plain
          // title grows above its box, a tenth of that height.
          : Align(
              alignment: Alignment.topLeft,
              widthFactor: bodySize / _flipTextSize,
              heightFactor: bodySize / _flipTextSize,
              child: FractionalTranslation(
                translation: Offset(
                  0,
                  -(_scale - 1) / 2 * bodySize / _flipTextSize,
                ),
                child: FlipText(
                  flipKey: flipKey!,
                  text: title,
                  scale: bodySize * _scale / _flipTextSize,
                  color: theme.inactiveColor,
                  alpha: (theme.inactiveColor.a * 255).roundToDouble(),
                ),
              ),
            ),
    );
  }
}

class QueryTracksPage extends StatefulWidget {
  final QueryList queries;
  final PlaybackMode? mode;

  /// A page with no title is drawn under the header, as the tracks page is.
  final String? title;

  /// Loads the tracks. Only a test gives another one.
  final QueryTrackPageLoader loadMixPage;

  /// Fires when the library's content changes. Only a test gives another one.
  final Stream<void>? contentChanged;

  const QueryTracksPage({
    super.key,
    required this.queries,
    required this.mode,
    required this.title,
    this.loadMixPage = queryMixTracks,
    this.contentChanged,
  });

  @override
  State<QueryTracksPage> createState() => _QueryTracksPageState();
}

class _QueryTracksPageState extends State<QueryTracksPage> {
  final _layoutManager = StartScreenLayoutManager();

  @override
  Widget build(BuildContext context) {
    final title = widget.title;

    return ChangeNotifierProvider<StartScreenLayoutManager>.value(
      value: _layoutManager,
      child: DeviceTypeBuilder(
        deviceType: const [DeviceType.dock, DeviceType.band, DeviceType.tv],
        builder: (context, activeBreakpoint) {
          final isMini = activeBreakpoint == DeviceType.dock ||
              activeBreakpoint == DeviceType.band;
          return PageContentFrame(
            top: isMini || title == null,
            // The title is at the top, below the status bar.
            child: SafeArea(
              left: false,
              right: false,
              bottom: false,
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  if (!isMini && title != null) QueryTracksTitle(title: title),
                  Expanded(
                    child: QueryTrackListView(
                      layoutManager: _layoutManager,
                      queries: widget.queries,
                      mode: widget.mode,
                      loadMixPage: widget.loadMixPage,
                      contentChanged: widget.contentChanged,
                    ),
                  ),
                ],
              ),
            ),
          );
        },
      ),
    );
  }
}
