import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:reorderables/reorderables.dart';

import '../../utils/settings_page_padding.dart';
import '../../utils/settings_body_padding.dart';
import '../../widgets/settings/settings_box_combo_box.dart';
import '../../widgets/unavailable_page_on_band.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../providers/library_home.dart';

class SettingsLibraryHome extends StatefulWidget {
  const SettingsLibraryHome({super.key});

  @override
  State<SettingsLibraryHome> createState() => _SettingsLibraryHomeState();
}

class _SettingsLibraryHomeState extends State<SettingsLibraryHome> {
  @override
  Widget build(BuildContext context) {
    final libraryHome = Provider.of<LibraryHomeProvider>(context);
    final theme = FluentTheme.of(context);

    return PageContentFrame(
      child: UnavailablePageOnBand(
        child: SettingsPagePadding(
          child: SingleChildScrollView(
            padding: getScrollContainerPadding(context),
            child: SettingsBodyPadding(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  SizedBox(
                    width: double.maxFinite,
                    child: ReorderableColumn(
                      needsLongPressDraggable: false,
                      onReorder: libraryHome.reorder,
                      children: libraryHome.entries
                          .map(
                            (item) => SettingsBoxComboBox(
                              key: ValueKey(item.id),
                              icon: item.definition.icon(context),
                              iconColor: item.value == 'disable'
                                  ? theme.resources.textFillColorDisabled
                                  : null,
                              title: item.definition.titleBuilder(context),
                              subtitle: item.definition.subtitleBuilder(context),
                              value: item.value ?? item.definition.defaultValue,
                              items: item.definition
                                  .optionBuilder(context)
                                  .map((x) => SettingsBoxComboBoxItem(
                                        value: x.$2,
                                        title: x.$1,
                                      ))
                                  .toList(),
                              onChanged: (newValue) {
                                libraryHome.updateEntryValue(
                                  item.id,
                                  newValue,
                                );
                              },
                            ),
                          )
                          .toList(),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
