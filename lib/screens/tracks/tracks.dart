import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/navigation_bar.dart';
import '../../widgets/playback_controller.dart';
import '../../screens/tracks/widgets/track_list.dart';

class TracksPage extends StatefulWidget {
  const TracksPage({super.key});

  @override
  State<TracksPage> createState() => _TracksPageState();
}

class _TracksPageState extends State<TracksPage> {
  @override
  Widget build(BuildContext context) {
    return const ScaffoldPage(
      content: Column(children: [
        NavigationBarPlaceholder(),
        Expanded(
          child: TrackListView(),
        ),
        PlaybackPlaceholder(),
      ]),
    );
  }
}
