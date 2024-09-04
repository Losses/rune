import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../config/animation.dart';
import '../../widgets/navigation_bar.dart';
import '../../widgets/playback_controller.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../screens/tracks/widgets/track_list.dart';

class TracksPage extends StatefulWidget {
  const TracksPage({super.key});

  @override
  State<TracksPage> createState() => _TracksPageState();
}

class _TracksPageState extends State<TracksPage> {
  final _layoutManager = StartScreenLayoutManager();

  @override
  void initState() {
    super.initState();

    Timer(Duration(milliseconds: gridAnimationDelay),
        () => _layoutManager.playAnimations());
  }

  @override
  Widget build(BuildContext context) {
    return ChangeNotifierProvider<StartScreenLayoutManager>.value(
        value: _layoutManager,
        child: const ScaffoldPage(
          content: Column(children: [
            NavigationBarPlaceholder(),
            Expanded(
              child: TrackListView(),
            ),
            PlaybackPlaceholder(),
          ]),
        ));
  }
}
