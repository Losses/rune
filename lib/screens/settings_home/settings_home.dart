import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../screens/settings_home/band_screen_settings_home_list.dart';
import '../../providers/responsive_providers.dart';

import 'large_screen_settings_home_list.dart';
import 'small_screen_settings_home_list.dart';

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
        child: BreakpointBuilder(
          breakpoints: const [
            DeviceType.band,
            DeviceType.dock,
            DeviceType.zune,
            DeviceType.tv
          ],
          builder: (context, activeBreakpoint) {
            if (activeBreakpoint == DeviceType.dock ||
                activeBreakpoint == DeviceType.band) {
              return BandScreenLibraryHomeListView(
                layoutManager: _layoutManager,
              );
            }

            if (activeBreakpoint == DeviceType.zune) {
              return SmallScreenSettingsHomeListView(
                layoutManager: _layoutManager,
              );
            }

            return LargeScreenSettingsHomeListView(
              layoutManager: _layoutManager,
            );
          },
        ),
      ),
    );
  }
}
