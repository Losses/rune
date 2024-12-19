import 'package:fluent_ui/fluent_ui.dart';

import 'settings_box_base.dart';

class SettingsBoxComboBoxItem<T> {
  final T value;
  final String title;

  const SettingsBoxComboBoxItem({
    required this.value,
    required this.title,
  });
}

class SettingsBoxComboBox<T> extends SettingsBoxBase {
  const SettingsBoxComboBox({
    super.key,
    required super.title,
    required super.subtitle,
    required this.value,
    required this.items,
    required this.onChanged,
    super.icon,
    super.iconColor,
  });

  final T value;
  final List<SettingsBoxComboBoxItem<T>> items;
  final Function(T?)? onChanged;

  @override
  Widget buildExpanderContent(BuildContext context) {
    return Column(
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
    );
  }

  @override
  Widget buildDefaultContent(BuildContext context) {
    return ComboBox<T>(
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
    );
  }
}
