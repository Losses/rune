import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/l10n.dart';

import 'settings_box_base.dart';

class SettingsBoxToggle extends StatelessWidget {
  const SettingsBoxToggle({
    super.key,
    required this.title,
    required this.subtitle,
    required this.value,
    required this.onChanged,
  });

  final String title;
  final String subtitle;

  final bool value;
  final Function(bool)? onChanged;

  Widget buildExpanderContent(BuildContext context) {
    final onChanged = this.onChanged;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 4),
          child: RadioButton(
            checked: value,
            content: Text(S.of(context).enable),
            onChanged: onChanged == null
                ? null
                : (isChecked) {
                    if (isChecked) onChanged(true);
                  },
          ),
        ),
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 4),
          child: RadioButton(
            checked: !value,
            content: Text(S.of(context).disable),
            onChanged: onChanged == null
                ? null
                : (isChecked) {
                    if (isChecked) onChanged(false);
                  },
          ),
        ),
      ],
    );
  }

  Widget buildDefaultContent(BuildContext context) {
    return ToggleSwitch(checked: value, onChanged: onChanged);
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
