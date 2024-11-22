import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:desktop_window/desktop_window.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:bitsdojo_window/bitsdojo_window.dart';

import '../../utils/l10n.dart';
import '../../utils/ax_shadow.dart';
import '../../utils/settings_manager.dart';
import '../../utils/update_color_mode.dart';
import '../../utils/theme_color_manager.dart';
import '../../utils/settings_page_padding.dart';
import '../../widgets/settings/settings_box_toggle.dart';
import '../../widgets/settings/settings_block_title.dart';
import '../../widgets/settings/settings_box_combo_box.dart';
import '../../widgets/tile/tile.dart';
import '../../widgets/unavailable_page_on_band.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';

import 'constants/colors.dart';
import 'constants/window_sizes.dart';

const colorModeKey = 'color_mode';
const themeColorKey = 'theme_color';
const disableBrandingAnimationKey = 'disable_branding_animation';
const enableDynamicColorsKey = 'enable_dynamic_color';
const windowSizeKey = 'window_size';

final _settingsManager = SettingsManager();

class SettingsTheme extends StatefulWidget {
  const SettingsTheme({super.key});

  @override
  State<SettingsTheme> createState() => _SettingsThemeState();
}

class _SettingsThemeState extends State<SettingsTheme> {
  Color? themeColor;
  bool? disableBrandingAnimation;
  bool? enableDynamicColors;
  String? windowSize;
  String colorMode = "system";

  @override
  void initState() {
    super.initState();
    _loadSettings();
  }

  Future<void> _loadSettings() async {
    final int? storedTheme =
        await _settingsManager.getValue<int>(themeColorKey);
    final String? storedColorMode =
        await _settingsManager.getValue<String>(colorModeKey);
    final bool? storedDisableBrandingAnimation =
        await _settingsManager.getValue<bool>(disableBrandingAnimationKey);
    final bool? storedEnableDynamicColors =
        await _settingsManager.getValue<bool>(enableDynamicColorsKey);
    final String? storedWindowSize =
        await _settingsManager.getValue<String>(windowSizeKey);

    setState(() {
      if (storedTheme != null) {
        themeColor = Color(storedTheme);
      }
      if (storedColorMode != null) {
        colorMode = storedColorMode;
      }
      if (disableBrandingAnimation != null) {
        disableBrandingAnimation = storedDisableBrandingAnimation;
      }
      if (storedEnableDynamicColors != null) {
        enableDynamicColors = storedEnableDynamicColors;
      }
      if (storedWindowSize != null) {
        windowSize = storedWindowSize;
      }
    });
  }

  Future<void> _updateThemeColor(Color? newThemeColor) async {
    setState(() {
      themeColor = newThemeColor;
      ThemeColorManager().updateUserSelectedColor(newThemeColor);
    });

    if (newThemeColor == null) {
      await SettingsManager().removeValue(themeColorKey);
    } else {
      await SettingsManager().setValue(themeColorKey, newThemeColor.value);
    }
  }

  void _handleDynamicColorToggle(bool value) async {
    await _settingsManager.setValue(enableDynamicColorsKey, value);

    setState(() {
      enableDynamicColors = value;
    });

    ThemeColorManager().updateDynamicColorSetting(value);
  }

  Future<void> _updateColorModeSetting(String newMode) async {
    setState(() {
      colorMode = newMode;

      updateColorMode(colorMode);
    });
    await SettingsManager().setValue(colorModeKey, newMode);
  }

  void _updateWindowSize(String newWindowSize) async {
    final firstView = WidgetsBinding.instance.platformDispatcher.views.first;

    final size = Platform.isWindows
        ? windowSizes[newWindowSize]! * firstView.devicePixelRatio
        : windowSizes[newWindowSize]!;

    setState(() {
      windowSize = newWindowSize;
    });

    await DesktopWindow.setWindowSize(size);
    appWindow.alignment = Alignment.center;
    await SettingsManager().setValue(windowSizeKey, newWindowSize);
  }

  @override
  Widget build(BuildContext context) {
    return PageContentFrame(
      child: UnavailablePageOnBand(
        child: SingleChildScrollView(
          padding: getScrollContainerPadding(context),
          child: SettingsPagePadding(
            child: Column(
              children: [
                SettingsBoxComboBox(
                  title: S.of(context).colorMode,
                  subtitle: S.of(context).colorModeSubtitle,
                  value: colorMode,
                  items: [
                    SettingsBoxComboBoxItem(
                      value: "system",
                      title: S.of(context).systemColorMode,
                    ),
                    SettingsBoxComboBoxItem(
                      value: "dark",
                      title: S.of(context).dark,
                    ),
                    SettingsBoxComboBoxItem(
                      value: "light",
                      title: S.of(context).light,
                    ),
                  ],
                  onChanged: (newValue) {
                    if (newValue != null) {
                      _updateColorModeSetting(newValue);
                    }
                  },
                ),
                Padding(
                  padding: const EdgeInsets.all(4),
                  child: Expander(
                    header: Padding(
                      padding: const EdgeInsets.symmetric(vertical: 11),
                      child: SettingsBlockTitle(
                        title: S.of(context).themeColor,
                        subtitle: S.of(context).themeColorSubtitle,
                      ),
                    ),
                    content: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Checkbox(
                          content: Text(S.of(context).followSystemTheme),
                          checked: themeColor == null,
                          onChanged: (isChecked) {
                            setState(() {
                              themeColor = null;
                              _updateThemeColor(null);
                            });
                          },
                        ),
                        const SizedBox(height: 12),
                        Wrap(
                          spacing: 2,
                          runSpacing: 2,
                          children: colors.entries.map((x) {
                            return SizedBox(
                              width: 40,
                              height: 40,
                              child: Tooltip(
                                message: x.key,
                                child: Tile(
                                  onPressed: () {
                                    _updateThemeColor(x.value);
                                  },
                                  child: Container(
                                    color: x.value,
                                    child: x.value == themeColor
                                        ? Icon(
                                            Symbols.check,
                                            color: Colors.white,
                                            shadows: axShadow(4),
                                          )
                                        : null,
                                  ),
                                ),
                              ),
                            );
                          }).toList(),
                        )
                      ],
                    ),
                  ),
                ),
                SettingsBoxToggle(
                  title: S.of(context).dynamicColors,
                  subtitle: S.of(context).dynamicColorsSubtitle,
                  value: enableDynamicColors ?? false,
                  onChanged: (value) {
                    _settingsManager.setValue(
                      enableDynamicColorsKey,
                      !value,
                    );

                    _handleDynamicColorToggle(value);
                  },
                ),
                SettingsBoxToggle(
                  title: S.of(context).brandingAnimation,
                  subtitle: S.of(context).brandingAnimationSubtitle,
                  value: !(disableBrandingAnimation ?? false),
                  onChanged: (value) {
                    _settingsManager.setValue(
                      disableBrandingAnimationKey,
                      !value,
                    );

                    setState(() {
                      disableBrandingAnimation = !value;
                    });
                  },
                ),
                SettingsBoxComboBox(
                  title: S.of(context).windowSize,
                  subtitle: S.of(context).windowSizeSubtitle,
                  value: windowSize ?? "normal",
                  items: [
                    SettingsBoxComboBoxItem(
                      value: "normal",
                      title: S.of(context).normalWindowSize,
                    ),
                    SettingsBoxComboBoxItem(
                      value: "slim",
                      title: S.of(context).slimWindowSize,
                    ),
                    SettingsBoxComboBoxItem(
                      value: "stocky",
                      title: S.of(context).stockyWindowSize,
                    ),
                  ],
                  onChanged: (newValue) {
                    if (newValue != null) {
                      _updateWindowSize(newValue);
                    }
                  },
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
