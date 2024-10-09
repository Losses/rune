import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import 'large_screen_track_list.dart';
import '../../widgets/navigation_bar/navigation_bar_placeholder.dart';
import '../../widgets/playback_controller/playback_placeholder.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';

class TracksPage extends StatefulWidget {
  const TracksPage({super.key});

  @override
  State<TracksPage> createState() => _TracksPageState();
}

class _TracksPageState extends State<TracksPage> {
  final _layoutManager = StartScreenLayoutManager();

  @override
  Widget build(BuildContext context) {
    return ChangeNotifierProvider<StartScreenLayoutManager>.value(
        value: _layoutManager,
        child: Column(children: [
          const NavigationBarPlaceholder(),
          Expanded(
            child: TrackListView(
              layoutManager: _layoutManager,
            ),
          ),
          const PlaybackPlaceholder(),
        ]));
  }
}
