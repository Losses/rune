import 'dart:async';

import 'package:player/widgets/start_screen/providers/start_screen_layout_manager.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/navigation_bar.dart';
import '../../widgets/playback_controller.dart';
import '../../providers/library_path.dart';

import 'library_home_list.dart';

class LibraryHomePage extends StatefulWidget {
  const LibraryHomePage({super.key});

  @override
  State<LibraryHomePage> createState() => _LibraryHomePageState();
}

class _LibraryHomePageState extends State<LibraryHomePage> {
  final _layoutManager = StartScreenLayoutManager();

  @override
  void initState() {
    super.initState();

    Timer(const Duration(milliseconds: 300),
        () => _layoutManager.playAnimations(0.1));
  }

  @override
  Widget build(BuildContext context) {
    final libraryPath = Provider.of<LibraryPathProvider>(context).currentPath;

    if (libraryPath == null) {
      return Container();
    }

    return ChangeNotifierProvider<StartScreenLayoutManager>.value(
        value: _layoutManager,
        child: ScaffoldPage(
          content: Column(children: [
            const NavigationBarPlaceholder(),
            Expanded(child: LibraryHomeListView(libraryPath: libraryPath)),
            const PlaybackPlaceholder()
          ]),
        ));
  }
}
