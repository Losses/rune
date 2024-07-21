import 'package:fluent_ui/fluent_ui.dart';
import 'package:player/screens/tracks/widgets/track_list.dart';

class TracksPage extends StatefulWidget {
  const TracksPage({super.key});

  @override
  State<TracksPage> createState() => _TracksPageState();
}

class _TracksPageState extends State<TracksPage> {
  @override
  Widget build(BuildContext context) {
    return const ScaffoldPage(
      header: PageHeader(
        title: Text('Tracks'),
      ),
      content: TrackListView(),
    );
  }
}
