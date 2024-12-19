import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/l10n.dart';

import 'settings_box_base.dart';

class SettingsBoxToggle extends SettingsBoxBase {
  const SettingsBoxToggle({
    super.key,
    required super.title,
    required super.subtitle,
    required this.value,
    required this.onChanged,
  });

  final bool value;
  final Function(bool)? onChanged;

  @override
  Widget buildExpanderContent(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 4),
          child: RadioButton(
            checked: value,
            content: Text(S.of(context).enable),
            onChanged: (isChecked) {
              if (isChecked && onChanged != null) {
                onChanged!(true);
              }
            },
          ),
        ),
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 4),
          child: RadioButton(
            checked: !value,
            content: Text(S.of(context).disable),
            onChanged: (isChecked) {
              if (isChecked && onChanged != null) {
                onChanged!(false);
              }
            },
          ),
        )
      ],
    );
  }

  @override
  Widget buildDefaultContent(BuildContext context) {
    return ToggleSwitch(
      checked: value,
      onChanged: (value) {
        if (onChanged != null) {
          onChanged!(value);
        }
      },
    );
  }
}
