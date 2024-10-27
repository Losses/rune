import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/ax_shadow.dart';
import '../../utils/settings_manager.dart';
import '../../utils/settings_page_padding.dart';
import '../../config/theme.dart';
import '../../widgets/tile/tile.dart';
import '../../widgets/unavailable_page_on_band.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';

import '../settings_playback/widgets/settings_block.dart';
import '../settings_playback/widgets/settings_block_title.dart';

const themeColorKey = 'theme_color';
const disableBrandingAnimationKey = 'disable_branding_animation';

final settingsManager = SettingsManager();

const Map<String, Color> colors = {
  "SAKURA": Color(0xFFFEDFE1),
  "MOMO": Color(0xFFF596AA),
  "KOHBAI": Color(0xFFE16B8C),
  "KARAKURENAI": Color(0xFFD0104C),
  "JINZAMOMI": Color(0xFFEB7A77),
  "AKABENI": Color(0xFFCB4042),
  "SANGOSYU": Color(0xFFF17C67),
  "BENIHI": Color(0xFFF75C2F),
  "ARAISYU": Color(0xFFFB966E),
  "TERIGAKI": Color(0xFFC46243),
  "ENSYUCHA": Color(0xFFCA7853),
  "OHNI": Color(0xFFF05E1C),
  "SHISHI": Color(0xFFF0A986),
  "AKASHIROTSURUBAMI": Color(0xFFE1A679),
  "AKAKUCHIBA": Color(0xFFC78550),
  "ARAIGAKI": Color(0xFFE79460),
  "KURUMI": Color(0xFF947A6D),
  "UMEZOME": Color(0xFFE9A368),
  "KUCHIBA": Color(0xFFE2943B),
  "GINSUSUTAKE": Color(0xFF82663A),
  "KUWACHA": Color(0xFFC99833),
  "YAMABUKI": Color(0xFFFFB11B),
  "TORINOKO": Color(0xFFDAC9A6),
  "SHIROTSURUBAMI": Color(0xFFDCB879),
  "TAMAGO": Color(0xFFF9BF45),
  "KUCHINASHI": Color(0xFFF6C555),
  "AKU": Color(0xFF877F6C),
  "KOKE": Color(0xFF838A2D),
  "HIWA": Color(0xFFBEC23F),
  "URAYANAGI": Color(0xFFB5CAA0),
  "YANAGIZOME": Color(0xFF91AD70),
  "HIWAMOEGI": Color(0xFF90B44B),
  "USUAO": Color(0xFF91B493),
  "AONI": Color(0xFF516E41),
  "NAE": Color(0xFF86C166),
  "MOEGI": Color(0xFF7BA23F),
  "BYAKUROKU": Color(0xFFA8D8B9),
  "MIDORI": Color(0xFF227D51),
  "WAKATAKE": Color(0xFF5DAC81),
  "TOKIWA": Color(0xFF1B813E),
  "AOTAKE": Color(0xFF00896C),
  "SABISEIJI": Color(0xFF86A697),
  "TOKUSA": Color(0xFF2D6D4B),
  "SEIHEKI": Color(0xFF268785),
  "DEED": Color(0xFFDEEDF1),
  "DORA": Color(0xFF00C8FF),
  "MIZUASAGI": Color(0xFF66BAB7),
  "SEIJI": Color(0xFF69B0AC),
  "HENKA": Color(0xFF28C8C8),
  "KAMENOZOKI": Color(0xFFA5DEE4),
  "BYAKUGUN": Color(0xFF78C2C4),
  "SHINBASHI": Color(0xFF0089A7),
  "MIZU": Color(0xFF81C7D4),
  "AINEZUMI": Color(0xFF566C73),
  "HANAASAGI": Color(0xFF1E88A8),
  "GUNJYO": Color(0xFF51A8DD),
  "WASURENAGUSA": Color(0xFF7DB9DE),
  "HANADA": Color(0xFF006284),
  "BENIKAKEHANA": Color(0xFF4E4F97),
  "KIKYO": Color(0xFF6A4C9C),
  "FUJIMURASAKI": Color(0xFF8A6BBE),
  "FUJI": Color(0xFF8B81C3),
  "KONKIKYO": Color(0xFF211E55),
  "SUMIRE": Color(0xFF66327C),
  "EDOMURASAKI": Color(0xFF77428D),
  "HASHITA": Color(0xFF986DB2),
  "USU": Color(0xFFB28FCE),
  "MESSHI": Color(0xFF533D5B),
  "BUDOHNEZUMI": Color(0xFF5E3D50),
  "AYAME": Color(0xFF6F3381),
};

class SettingsTheme extends StatefulWidget {
  const SettingsTheme({super.key});

  @override
  State<SettingsTheme> createState() => _SettingsThemeState();
}

class _SettingsThemeState extends State<SettingsTheme> {
  Color? themeColor;
  bool? disableBrandingAnimation;

  @override
  void initState() {
    super.initState();
    _loadSettings();
  }

  Future<void> _loadSettings() async {
    final int? storedTheme = await settingsManager.getValue<int>(themeColorKey);
    final bool? storedDisableBrandingAnimation =
        await settingsManager.getValue<bool>(disableBrandingAnimationKey);

    if (storedTheme != null) {
      setState(() {
        themeColor = Color(storedTheme);
        disableBrandingAnimation = storedDisableBrandingAnimation;
      });
    }
  }

  Future<void> _updateThemeColor(Color? newThemeColor) async {
    setState(() {
      themeColor = newThemeColor;

      appTheme.updateThemeColor(themeColor);
    });

    if (newThemeColor == null) {
      await SettingsManager().removeValue(themeColorKey);
    } else {
      await SettingsManager().setValue(themeColorKey, newThemeColor.value);
    }
  }

  @override
  Widget build(BuildContext context) {
    return PageContentFrame(
      child: UnavailablePageOnBand(
        child: SingleChildScrollView(
          child: SettingsPagePadding(
            child: Column(
              children: [
                Padding(
                  padding: const EdgeInsets.all(4),
                  child: Expander(
                    header: const Padding(
                      padding: EdgeInsets.symmetric(vertical: 11),
                      child: SettingsBlockTitle(
                        title: "Theme Color",
                        subtitle:
                            "Select a color to give your Rune a refreshed look and feel.",
                      ),
                    ),
                    content: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Checkbox(
                          content: const Text("Follow system theme"),
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
                SettingsBlock(
                  title: "Branding Animation",
                  subtitle: "Play branding animation when Rune starts.",
                  child: ToggleSwitch(
                    checked: !(disableBrandingAnimation ?? false),
                    onChanged: (value) {
                      settingsManager.setValue(
                        disableBrandingAnimationKey,
                        !value,
                      );

                      setState(() {
                        disableBrandingAnimation = !value;
                      });
                    },
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
