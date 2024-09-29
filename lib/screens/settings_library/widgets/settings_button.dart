import 'package:fluent_ui/fluent_ui.dart';

import 'settings_tile_title.dart';

class SettingsButton extends StatelessWidget {
  const SettingsButton({
    super.key,
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.onPressed,
    this.suffixIcon,
  });

  final IconData icon;
  final IconData? suffixIcon;
  final String title;
  final String subtitle;
  final void Function()? onPressed;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.all(4),
      child: Button(
        style: ButtonStyle(
          shape: WidgetStateProperty.all(
            RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(4),
            ),
          ),
        ),
        onPressed: onPressed,
        child: SettingsTileTitle(
          icon: icon,
          suffixIcon: suffixIcon,
          title: title,
          subtitle: subtitle,
          showActions: false,
          actionsBuilder: (context) => Container(),
        ),
      ),
    );
  }
}
