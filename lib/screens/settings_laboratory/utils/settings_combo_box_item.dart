import 'package:fluent_ui/fluent_ui.dart';

class SettingsComboBoxItem<T> {
  final T value;
  final IconData? icon;
  final String text;

  const SettingsComboBoxItem({
    required this.value,
    this.icon,
    required this.text,
  });
}
