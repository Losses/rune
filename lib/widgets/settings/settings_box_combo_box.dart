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

class SettingsBoxComboBox<T> extends StatelessWidget {
  const SettingsBoxComboBox({
    super.key,
    required this.title,
    required this.subtitle,
    required this.value,
    required this.items,
    required this.onChanged,
    this.icon,
    this.iconColor,
  });

  final String title;
  final String subtitle;
  final IconData? icon;
  final Color? iconColor;

  final T value;
  final List<SettingsBoxComboBoxItem<T>> items;
  final Function(T?)? onChanged;

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

  @override
  Widget build(BuildContext context) {
    return SettingsBoxBase(
      title: title,
      subtitle: subtitle,
      buildExpanderContent: buildExpanderContent,
      buildDefaultContent: buildDefaultContent,
    );
  }
}
