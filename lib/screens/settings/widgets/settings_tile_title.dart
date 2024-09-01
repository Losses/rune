import 'package:fluent_ui/fluent_ui.dart';

class SettingsTileTitle extends StatelessWidget {
  final IconData icon;
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
  });

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
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
            child: Icon(icon, size: 26),
          ),
        ),
        const SizedBox(
          width: 12,
        ),
        Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const SizedBox(
              height: 8,
            ),
            Text(title,
                style: theme.typography.body?.apply(fontSizeFactor: 1.1)),
            const SizedBox(
              height: 2,
            ),
            Text(
              subtitle,
              style: theme.typography.caption?.apply(
                color: theme.activeColor.withAlpha(160),
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
      ],
    );
  }
}
