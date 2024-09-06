import 'package:fluent_ui/fluent_ui.dart';

import './albums_list.dart';

import '../../widgets/navigation_bar.dart';
import '../../widgets/playback_controller.dart';

class AlbumsPage extends StatefulWidget {
  const AlbumsPage({super.key});

  @override
  State<AlbumsPage> createState() => _AlbumsPageState();
}

class _AlbumsPageState extends State<AlbumsPage> {
  @override
  Widget build(BuildContext context) {
    return const Column(children: [
      NavigationBarPlaceholder(),
      Expanded(child: AlbumsListView()),
      PlaybackPlaceholder()
    ]);
  }
}
