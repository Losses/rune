import 'package:fluent_ui/fluent_ui.dart';

import 'library_home_list.dart';

import '../../widgets/navigation_bar.dart';
import '../../widgets/playback_controller.dart';

class LibraryHomePage extends StatefulWidget {
  const LibraryHomePage({super.key});

  @override
  State<LibraryHomePage> createState() => _LibraryHomePageState();
}

class _LibraryHomePageState extends State<LibraryHomePage> {
  @override
  Widget build(BuildContext context) {
    return const ScaffoldPage(
      content: Column(children: [
        NavigationBarPlaceholder(),
        Expanded(child: LibraryHomeListView()),
        PlaybackPlaceholder()
      ]),
    );
  }
}
