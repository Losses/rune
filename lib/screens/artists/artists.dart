import 'package:fluent_ui/fluent_ui.dart';

import './artists_list.dart';

import '../../widgets/playback_controller.dart';

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
