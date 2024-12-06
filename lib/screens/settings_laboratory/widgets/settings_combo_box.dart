import 'package:fluent_ui/fluent_ui.dart';

import '../utils/settings_combo_box_item.dart';

class SettingsComboBox<T> extends StatelessWidget {
  final T? value;
  final List<SettingsComboBoxItem<T>> items;
  final ValueChanged<T?> onChanged;

  const SettingsComboBox({
    super.key,
    required this.value,
    required this.items,
    required this.onChanged,
  });

  @override
  Widget build(BuildContext context) {
    return ComboBox<T>(
      value: value,
      items: items.map((e) {
        return ComboBoxItem<T>(
          value: e.value,
          child: Row(
            children: [
              if (e.icon != null) ...[
                Icon(e.icon),
                SizedBox(width: 4),
              ],
              Text(e.text,
                  textAlign: TextAlign.start, overflow: TextOverflow.ellipsis),
            ],
          ),
        );
      }).toList(),
      onChanged: onChanged,
    );
  }
}
