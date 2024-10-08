import 'package:player/screens/library_home/small_screen_library_home_list.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:responsive_framework/responsive_framework.dart';

import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../widgets/navigation_bar/navigation_bar_placeholder.dart';
import '../../widgets/playback_controller/playback_placeholder.dart';
import '../../providers/library_path.dart';

import 'large_screen_library_home_list.dart';

class LibraryHomePage extends StatefulWidget {
  const LibraryHomePage({super.key});

  @override
  State<LibraryHomePage> createState() => _LibraryHomePageState();
}

class _LibraryHomePageState extends State<LibraryHomePage> {
  final _layoutManager = StartScreenLayoutManager();

  @override
  void dispose() {
    super.dispose();
    _layoutManager.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final libraryPath = Provider.of<LibraryPathProvider>(context).currentPath;

    if (libraryPath == null) {
      return Container();
    }

    final isSmallPhone =
        ResponsiveBreakpoints.of(context).smallerOrEqualTo('SMALL_PHONE');

    return ChangeNotifierProvider<StartScreenLayoutManager>.value(
      value: _layoutManager,
      child: Column(
        children: [
          const NavigationBarPlaceholder(),
          Expanded(
            child: !isSmallPhone
                ? LargeScreenLibraryHomeListView(
                    libraryPath: libraryPath,
                    layoutManager: _layoutManager,
                  )
                : SmallScreenLibraryHomeListView(
                    libraryPath: libraryPath,
                    layoutManager: _layoutManager,
                  ),
          ),
          const PlaybackPlaceholder()
        ],
      ),
    );
  }
}
