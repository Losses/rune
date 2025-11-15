import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/settings_page_padding.dart';
import '../../widgets/unavailable_page_on_band.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';

import 'widgets/color_mode_setting.dart';
import 'widgets/theme_color_setting.dart';
import 'widgets/window_size_setting.dart';
import 'widgets/dynamic_colors_setting.dart';
import 'widgets/branding_animation_setting.dart';
import 'widgets/remember_window_size_setting_state.dart';
import 'widgets/linux_custom_window_controls_setting.dart';

class SettingsTheme extends StatefulWidget {
  const SettingsTheme({super.key});

  @override
  State<SettingsTheme> createState() => _SettingsThemeState();
}

class _SettingsThemeState extends State<SettingsTheme> {
  @override
  Widget build(BuildContext context) {
    return PageContentFrame(
      child: UnavailablePageOnBand(
        child: SingleChildScrollView(
          padding: getScrollContainerPadding(context),
          child: SettingsPagePadding(
            child: Column(
              children: [
                ColorModeSetting(),
                ThemeColorSetting(),
                DynamicColorsSetting(),
                BrandingAnimationSetting(),
                WindowSizeSetting(),
                RememberWindowSizeSetting(),
                LinuxCustomWindowControlsSetting(),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
