import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_staggered_grid_view/flutter_staggered_grid_view.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';

import '../../utils/l10n.dart';
import '../../utils/settings_manager.dart';
import '../../utils/router/navigation.dart';
import '../../utils/dialogs/export_cover_wall/show_export_cover_wall_dialog.dart';
import '../../widgets/cover_wall_background/cover_wall_background.dart';
import '../../widgets/settings/settings_container.dart';
import '../../widgets/router/rune_stack.dart';
import '../../providers/responsive_providers.dart';

import 'constants/cover_wall_item_count.dart';

class SettingsLaboratory extends StatefulWidget {
  const SettingsLaboratory({super.key});

  @override
  State<SettingsLaboratory> createState() => _SettingsLaboratoryState();
}

class _SettingsLaboratoryState extends State<SettingsLaboratory> {
  String? randomCoverWallCount = '40';

  final SettingsManager _settingsManager = SettingsManager();

  @override
  initState() {
    super.initState();
    _loadSettings();
  }

  Future<void> _loadSettings() async {
    final String? storedRandomCoverWallCount =
        await _settingsManager.getValue<String>(randomCoverWallCountKey);

    setState(() {
      if (storedRandomCoverWallCount != null) {
        randomCoverWallCount = storedRandomCoverWallCount;
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    final typography = FluentTheme.of(context).typography;

    return RuneStack(
      children: [
        Positioned(
          top: 16,
          left: 16,
          child: IconButton(
            icon: Icon(
              Symbols.arrow_back,
              size: 24,
            ),
            onPressed: () => $pop(),
          ),
        ),
        Align(
          alignment: Alignment.topCenter,
          child: Padding(
            padding: EdgeInsets.only(top: 20),
            child: Text(
              S.of(context).laboratory,
              style: typography.title,
            ),
          ),
        ),
        Align(
          alignment: Alignment.topCenter,
          child: Container(
            padding: EdgeInsets.symmetric(horizontal: 8.0),
            constraints: BoxConstraints(maxWidth: 800),
            child: Padding(
              padding: EdgeInsets.only(top: 68),
              child: SmallerOrEqualTo(
                deviceType: DeviceType.phone,
                builder: (context, isMini) => MasonryGridView(
                  padding: EdgeInsets.only(top: 4),
                  gridDelegate: SliverSimpleGridDelegateWithFixedCrossAxisCount(
                    crossAxisCount: isMini ? 1 : 2,
                  ),
                  mainAxisSpacing: 4,
                  crossAxisSpacing: 4,
                  children: [
                    SettingsContainer(
                      margin: EdgeInsets.all(0),
                      padding: EdgeInsets.all(8),
                      child: Column(
                        children: [
                          Text(
                            "Cover Wall Richness",
                            style: typography.subtitle,
                          ),
                          SizedBox(height: 8),
                          Text(
                            "Customize the maximum number of covers displayed on the cover wall. Please note that having too many may lead to insufficient operating system memory.",
                            style: TextStyle(height: 1.4),
                          ),
                          SizedBox(height: 16),
                          ComboBox<String>(
                            value: randomCoverWallCount,
                            items: randomCoverWallCountConfig(context).map((e) {
                              return ComboBoxItem(
                                value: e.value,
                                child: Row(
                                  children: [
                                    Icon(e.icon),
                                    SizedBox(width: 4),
                                    Text(
                                      e.title,
                                      textAlign: TextAlign.start,
                                      overflow: TextOverflow.ellipsis,
                                    )
                                  ],
                                ),
                              );
                            }).toList(),
                            onChanged: (x) => setState(() {
                              if (x == null) return;

                              randomCoverWallCount = x;
                              _settingsManager.setValue<String>(
                                randomCoverWallCountKey,
                                x,
                              );
                            }),
                          ),
                        ],
                      ),
                    ),
                    SettingsContainer(
                      margin: EdgeInsets.all(0),
                      padding: EdgeInsets.all(8),
                      child: Column(
                        children: [
                          Text(
                            "Library Cover Wallpaper",
                            style: typography.subtitle,
                          ),
                          SizedBox(height: 8),
                          Text(
                            "Render a cover wall that includes all tracks. Please be aware that this feature may cause the software to crash due to insufficient available memory.",
                            style: TextStyle(height: 1.4),
                          ),
                          SizedBox(height: 16),
                          FilledButton(
                            onPressed: () {
                              showExportCoverWallDialog(
                                context,
                                [("lib::directory.deep", "/")],
                                "Rune",
                              );
                            },
                            child: Text("Getting Started"),
                          )
                        ],
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ],
    );
  }
}
