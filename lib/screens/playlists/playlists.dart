import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/navigation_bar.dart';
import '../../widgets/playback_controller.dart';

import 'playlists_list.dart';

class PlaylistsPage extends StatefulWidget {
  const PlaylistsPage({super.key});

  @override
  State<PlaylistsPage> createState() => _PlaylistsPageState();
}

class _PlaylistsPageState extends State<PlaylistsPage> {
  @override
  Widget build(BuildContext context) {
    return const ScaffoldPage(
      content: Column(children: [
        NavigationBarPlaceholder(),
        Expanded(child: PlaylistsListView()),
        PlaybackPlaceholder()
      ]),
    );
  }
}
