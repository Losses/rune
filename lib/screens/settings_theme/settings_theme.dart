import 'dart:math' as math;

import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/navigation_bar/page_content_frame.dart';

import 'widgets/color_mode_setting.dart';
import 'widgets/interface_scale_setting.dart';
import 'widgets/theme_color_setting.dart';
import 'widgets/window_size_setting.dart';
import 'widgets/dynamic_colors_setting.dart';
import 'widgets/branding_animation_setting.dart';
import 'widgets/remember_window_size_setting_state.dart';
import 'widgets/linux_custom_window_controls_setting.dart';
import 'widgets/cover_wall_display_setting.dart';
import 'widgets/visualizer_effect_level_setting.dart';
import 'widgets/visualizer_preset_setting.dart';

class SettingsTheme extends StatefulWidget {
  const SettingsTheme({super.key});

  @override
  State<SettingsTheme> createState() => _SettingsThemeState();
}

class _SettingsThemeState extends State<SettingsTheme> {
  @override
  Widget build(BuildContext context) {
    return PageContentFrame(
      left: false,
      child: LayoutBuilder(
        builder: (context, constraints) => SingleChildScrollView(
          scrollDirection: Axis.horizontal,
          child: SizedBox(
            width: math.max(220, constraints.maxWidth),
            child: SingleChildScrollView(
              padding: getScrollContainerPadding(context),
              child: Padding(
                padding: EdgeInsets.symmetric(
                  horizontal: constraints.maxWidth < 300 ? 4 : 24,
                ),
                child: Column(
                  children: [
                    InterfaceScaleSetting(),
                    ColorModeSetting(),
                    ThemeColorSetting(),
                    DynamicColorsSetting(),
                    BrandingAnimationSetting(),
                    WindowSizeSetting(),
                    RememberWindowSizeSetting(),
                    LinuxCustomWindowControlsSetting(),
                    CoverWallDisplaySetting(),
                    VisualizerPresetSetting(),
                    VisualizerEffectLevelSetting(),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
