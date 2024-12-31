import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/l10n.dart';
import '../../utils/settings_manager.dart';
import '../../widgets/belt_container.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../widgets/turntile/small_screen_feature_list.dart';
import '../../widgets/band_screen/band_screen_feature_list.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../providers/responsive_providers.dart';

import 'constants/first_column.dart';
import 'large_screen_settings_home_list.dart';

class SettingsHomePage extends StatefulWidget {
  static const mysteriousKey = 'mysterious_key';

  const SettingsHomePage({super.key});

  @override
  State<SettingsHomePage> createState() => _SettingsHomePageState();
}

class _SettingsHomePageState extends State<SettingsHomePage> {
  final _layoutManager = StartScreenLayoutManager();

  bool? mysterious;

  @override
  void initState() {
    super.initState();

    SettingsManager().getValue<bool>(SettingsHomePage.mysteriousKey).then((x) {
      if (!context.mounted) return;

      setState(() {
        mysterious = x;
      });
    });
  }

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
                    items: [
                      ...firstColumn,
                      if (mysterious == true)
                        (
                          (context) => S.of(context).laboratory,
                          '/settings/laboratory',
                          Symbols.interests,
                          false
                        )
                    ],
                    layoutManager: _layoutManager,
                    topPadding: !isDock,
                  );
                }

                return LargeScreenSettingsHomeListView(
                  layoutManager: _layoutManager,
                  topPadding: !isDock,
                  mysterious: mysterious,
                );
              },
            );
          },
        ),
      ),
    );
  }
}
