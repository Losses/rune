import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/band_screen/band_screen_feature_list.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../widgets/turntile/small_screen_feature_list.dart';
import '../../widgets/belt_container.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../providers/library_path.dart';
import '../../providers/responsive_providers.dart';

import 'constants/first_column.dart';
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

    return ChangeNotifierProvider<StartScreenLayoutManager>.value(
      value: _layoutManager,
      child: SmallerOrEqualTo(
        deviceType: DeviceType.dock,
        builder: (context, isDock) {
          return PageContentFrame(
            top: !isDock,
            left: false,
            child: DeviceTypeBuilder(
              deviceType: const [
                DeviceType.band,
                DeviceType.belt,
                DeviceType.dock,
                DeviceType.zune,
                DeviceType.tv
              ],
              builder: (context, activeBreakpoint) {
                if (activeBreakpoint == DeviceType.belt) {
                  return BeltContainer(
                    child: BandScreenFeatureList(
                      items: bandScreenFirstColumn,
                      layoutManager: _layoutManager,
                      topPadding: !isDock,
                    ),
                  );
                }

                if (activeBreakpoint == DeviceType.dock ||
                    activeBreakpoint == DeviceType.band) {
                  return BandScreenFeatureList(
                    items: bandScreenFirstColumn,
                    layoutManager: _layoutManager,
                    topPadding: !isDock,
                  );
                }

                if (activeBreakpoint == DeviceType.zune) {
                  return SmallScreenFeatureListView(
                    items: smallScreenFirstColumn,
                    layoutManager: _layoutManager,
                    topPadding: !isDock,
                  );
                }

                return LargeScreenLibraryHomeListView(
                  libraryPath: libraryPath,
                  layoutManager: _layoutManager,
                  topPadding: !isDock,
                );
              },
            ),
          );
        },
      ),
    );
  }
}
