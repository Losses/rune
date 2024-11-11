import 'package:fluent_ui/fluent_ui.dart';

import '../../providers/responsive_providers.dart';

import 'settings_block.dart';
import 'settings_block_title.dart';

class SettingsBoxComboBoxItem<T> {
  final T value;
  final String title;

  const SettingsBoxComboBoxItem({
    required this.value,
    required this.title,
  });
}

class SettingsBoxComboBox<T> extends StatelessWidget {
  const SettingsBoxComboBox({
    super.key,
    required this.title,
    required this.subtitle,
    required this.value,
    required this.items,
    required this.onChanged,
  });

  final String title;
  final String subtitle;
  final T value;
  final List<SettingsBoxComboBoxItem<T>> items;
  final Function(T?)? onChanged;

  @override
  Widget build(BuildContext context) {
    return SmallerOrEqualTo(
      deviceType: DeviceType.zune,
      builder: (context, isZune) {
        if (isZune) {
          return Padding(
            padding: const EdgeInsets.all(4),
            child: Expander(
              header: Padding(
                padding: const EdgeInsets.symmetric(vertical: 11),
                child: SettingsBlockTitle(
                  title: title,
                  subtitle: subtitle,
                ),
              ),
              content: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: items
                    .map(
                      (x) => Padding(
                        padding: const EdgeInsets.symmetric(vertical: 4),
                        child: RadioButton(
                          checked: value == x.value,
                          content: Text(x.title),
                          onChanged: (isChecked) {
                            if (isChecked && onChanged != null) {
                              onChanged!(x.value);
                            }
                          },
                        ),
                      ),
                    )
                    .toList(),
              ),
            ),
          );
        }
        return SettingsBlock(
          title: title,
          subtitle: subtitle,
          child: ComboBox<T>(
            value: value,
            items: items
                .map(
                  (x) => ComboBoxItem<T>(
                    value: x.value,
                    child: Text(x.title),
                  ),
                )
                .toList(),
            onChanged: onChanged,
          ),
        );
      },
    );
  }
}
