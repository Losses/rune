import 'package:fluent_ui/fluent_ui.dart';

import './albums_list.dart';

import '../../widgets/playback_controller.dart';

class AlbumsPage extends StatefulWidget {
  const AlbumsPage({super.key});

  @override
  State<AlbumsPage> createState() => _AlbumsPageState();
}

class _AlbumsPageState extends State<AlbumsPage> {
  @override
  Widget build(BuildContext context) {
    return const ScaffoldPage(
      header: PageHeader(
        title: Text('Albums'),
      ),
      content: Column(
          children: [Expanded(child: AlbumsListView()), PlaybackPlaceholder()]),
    );
  }
}
