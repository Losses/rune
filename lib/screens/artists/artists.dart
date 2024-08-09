import 'package:fluent_ui/fluent_ui.dart';
import 'package:player/screens/artists/artists_list.dart';
import 'package:player/widgets/playback_controller.dart';

class ArtistsPage extends StatefulWidget {
  const ArtistsPage({super.key});

  @override
  State<ArtistsPage> createState() => _ArtistsPageState();
}

class _ArtistsPageState extends State<ArtistsPage> {
  @override
  Widget build(BuildContext context) {
    return const ScaffoldPage(
      header: PageHeader(
        title: Text('Artists'),
      ),
      content: Column(children: [
        Expanded(child: ArtistsListView()),
        PlaybackPlaceholder()
      ]),
    );
  }
}
