import 'package:fluent_ui/fluent_ui.dart';

class SettingsTileTitle extends StatelessWidget {
  final IconData icon;
  final IconData? suffixIcon;
  final String title;
  final String subtitle;
  final bool showActions;
  final Widget Function(BuildContext context) actionsBuilder;

  const SettingsTileTitle({
    super.key,
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.showActions,
    required this.actionsBuilder,
    this.suffixIcon,
  });

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return Row(
      crossAxisAlignment: CrossAxisAlignment.center,
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
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
        const SizedBox(
          width: 12,
        ),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                title,
                style: theme.typography.body?.apply(fontSizeFactor: 1.1),
              ),
              const SizedBox(
                height: 2,
              ),
              Text(
                subtitle,
                style: theme.typography.caption?.apply(
                  color: theme.inactiveColor.withAlpha(160),
                ),
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
  }
}
