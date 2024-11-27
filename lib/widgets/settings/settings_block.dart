import 'package:fluent_ui/fluent_ui.dart';

import '../../providers/responsive_providers.dart';

import 'settings_block_title.dart';
import 'settings_container.dart';

class SettingsBlock extends StatelessWidget {
  const SettingsBlock({
    super.key,
    this.icon,
    this.iconColor,
    required this.title,
    required this.subtitle,
    required this.child,
    this.radius = 4,
  });

  final IconData? icon;
  final Color? iconColor;
  final String title;
  final String subtitle;
  final Widget child;
  final double radius;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return SettingsContainer(
      child: SmallerOrEqualToScreenSize(
        maxSize: 200,
        builder: (context, isMini) {
          return Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              if (!isMini && icon != null)
                Container(
                  width: 36,
                  height: 36,
                  decoration: BoxDecoration(
                    color: iconColor ?? theme.accentColor,
                    borderRadius: BorderRadius.circular(2),
                  ),
                  child: Icon(
                    icon,
                    color: theme.activeColor,
                    size: 26,
                  ),
                ),
              if (!isMini && icon != null) const SizedBox(width: 12),
              if (isMini && icon != null) const SizedBox(height: 48, width: 4),
              Expanded(
                child: SettingsBlockTitle(
                  title: title,
                  subtitle: subtitle,
                ),
              ),
              const SizedBox(width: 8),
              child,
            ],
          );
        },
      ),
    );
  }
}
