import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/ax_shadow.dart';
import '../../../utils/l10n.dart';
import '../../../utils/theme_color_manager.dart';
import '../../../widgets/settings/settings_block_title.dart';
import '../../../widgets/tile/tile.dart';
import '../../../constants/configurations.dart';
import '../../../constants/settings_manager.dart';

import '../constants/colors.dart';

class ThemeColorSetting extends StatefulWidget {
  const ThemeColorSetting({super.key});

  @override
  ThemeColorSettingState createState() => ThemeColorSettingState();
}

class ThemeColorSettingState extends State<ThemeColorSetting> {
  Color? themeColor;

  @override
  void initState() {
    super.initState();
    _loadThemeColor();
  }

  Future<void> _loadThemeColor() async {
    final storedTheme = await $settingsManager.getValue<int>(kThemeColorKey);
    setState(() {
      themeColor = storedTheme != null ? Color(storedTheme) : null;
    });
  }

  Future<void> _updateThemeColor(Color? newThemeColor) async {
    setState(() {
      themeColor = newThemeColor;
      ThemeColorManager().updateUserSelectedColor(newThemeColor);
    });

    if (newThemeColor == null) {
      await $settingsManager.removeValue(kThemeColorKey);
    } else {
      await $settingsManager.setValue(kThemeColorKey, newThemeColor.value);
    }
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    return Padding(
      padding: const EdgeInsets.all(4),
      child: Expander(
        header: Padding(
          padding: const EdgeInsets.symmetric(vertical: 11),
          child: SettingsBlockTitle(
            title: s.themeColor,
            subtitle: s.themeColorSubtitle,
          ),
        ),
        content: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Checkbox(
              content: Text(s.followSystemTheme),
              checked: themeColor == null,
              onChanged: (isChecked) {
                _updateThemeColor(null);
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
                            ? Icon(Symbols.check,
                                color: Colors.white, shadows: axShadow(4))
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
    );
  }
}
