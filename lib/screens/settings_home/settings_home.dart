import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/belt_container.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../widgets/turntile/small_screen_feature_list.dart';
import '../../widgets/band_screen/band_screen_feature_list.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../providers/responsive_providers.dart';

import 'constants/first_column.dart';
import 'large_screen_settings_home_list.dart';

class SettingsHomePage extends StatefulWidget {
  const SettingsHomePage({super.key});

  @override
  State<SettingsHomePage> createState() => _SettingsHomePageState();
}

class _SettingsHomePageState extends State<SettingsHomePage> {
  final _layoutManager = StartScreenLayoutManager();

  @override
  void dispose() {
    super.dispose();
    _layoutManager.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return ChangeNotifierProvider<StartScreenLayoutManager>.value(
      value: _layoutManager,
      child: PageContentFrame(
        child: SmallerOrEqualTo(
          deviceType: DeviceType.dock,
          builder: (context, isDock) {
            return DeviceTypeBuilder(
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
                      items: firstColumn,
                      layoutManager: _layoutManager,
                      topPadding: !isDock,
                    ),
                  );
                }

                if (activeBreakpoint == DeviceType.dock ||
                    activeBreakpoint == DeviceType.band) {
                  return BandScreenFeatureList(
                    items: firstColumn,
                    layoutManager: _layoutManager,
                    topPadding: !isDock,
                  );
                }

                if (activeBreakpoint == DeviceType.zune) {
                  return SmallScreenFeatureListView(
                    items: firstColumn,
                    layoutManager: _layoutManager,
                    topPadding: !isDock,
                  );
                }

                return LargeScreenSettingsHomeListView(
                  layoutManager: _layoutManager,
                  topPadding: !isDock,
                );
              },
            );
          },
        ),
      ),
    );
  }
}
