import 'package:fluent_ui/fluent_ui.dart';

import '../../generated/l10n.dart';
import '../../providers/responsive_providers.dart';

import 'settings_block.dart';
import 'settings_block_title.dart';

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
              ),
            ),
          );
        }
        return SettingsBlock(
          title: title,
          subtitle: subtitle,
          child: ToggleSwitch(
            checked: value,
            onChanged: (value) {
              if (onChanged != null) {
                onChanged!(value);
              }
            },
          ),
        );
      },
    );
  }
}
