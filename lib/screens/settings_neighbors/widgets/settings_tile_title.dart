import 'package:fluent_ui/fluent_ui.dart';

import '../../../providers/responsive_providers.dart';

class SettingsTileTitle extends StatelessWidget {
  final IconData icon;
  final IconData? suffixIcon;
  final String title;
  final String subtitle;
  final bool showActions;
  final Widget Function(BuildContext context) actionsBuilder;
  final bool wrap;

  const SettingsTileTitle({
    super.key,
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.showActions,
    required this.actionsBuilder,
    this.suffixIcon,
    this.wrap = false,
  });

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return SmallerOrEqualToScreenSize(
        maxSize: 212,
        builder: (context, isMini) {
          return Row(
            crossAxisAlignment: CrossAxisAlignment.center,
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              if (!isMini)
                Padding(
                  padding: const EdgeInsets.symmetric(vertical: 8),
                  child: Container(
                    width: 36,
                    height: 36,
                    decoration: BoxDecoration(
                      color: theme.accentColor,
                      borderRadius: BorderRadius.circular(2),
                    ),
                    child: Icon(icon, color: theme.activeColor, size: 26),
                  ),
                ),
              if (!isMini)
                const SizedBox(
                  width: 12,
                ),
              if (isMini) const SizedBox(height: 48, width: 4),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  mainAxisAlignment: MainAxisAlignment.center,
                  children: [
                    Text(
                      title,
                      style: theme.typography.body?.apply(fontSizeFactor: 1.1),
                      overflow: TextOverflow.ellipsis,
                      textAlign: TextAlign.start,
                    ),
                    const SizedBox(
                      height: 2,
                    ),
                    Text(
                      subtitle,
                      style: theme.typography.caption?.apply(
                        color: theme.inactiveColor.withAlpha(160),
                      ),
                      overflow: wrap ? null : TextOverflow.ellipsis,
                      textAlign: TextAlign.start,
                    ),
                    if (showActions) ...[
                      const SizedBox(
                        height: 12,
                      ),
                      actionsBuilder(context),
                      const SizedBox(
                        height: 8,
                      ),
                    ],
                  ],
                ),
              ),
              if (suffixIcon != null)
                Icon(
                  suffixIcon,
                  color: theme.inactiveColor.withAlpha(160),
                  size: 20,
                )
            ],
          );
        });
  }
}
