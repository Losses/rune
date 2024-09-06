import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/navigation_bar.dart';
import '../../widgets/playback_controller.dart';

import './artists_list.dart';

class ArtistsPage extends StatefulWidget {
  const ArtistsPage({super.key});

  @override
  State<ArtistsPage> createState() => _ArtistsPageState();
}

class _ArtistsPageState extends State<ArtistsPage> {
  @override
  Widget build(BuildContext context) {
    return const Column(children: [
      NavigationBarPlaceholder(),
      Expanded(child: ArtistsListView()),
      PlaybackPlaceholder()
    ]);
  }
}
