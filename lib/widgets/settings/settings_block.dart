import 'package:fluent_ui/fluent_ui.dart';

import '../../providers/responsive_providers.dart';

import 'settings_block_title.dart';

class SettingsBlock extends StatefulWidget {
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
  SettingsBlockState createState() => SettingsBlockState();
}

class SettingsBlockState extends State<SettingsBlock> {
  bool _isHovered = false;

  final FocusNode _focusNode = FocusNode(debugLabel: 'Tile');

  @override
  void dispose() {
    super.dispose();
    _focusNode.dispose();
  }

  void _onEnter(event) {
    setState(() {
      _isHovered = true;
    });
  }

  void _onExit(event) {
    setState(() {
      _isHovered = false;
    });
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return Padding(
      padding: const EdgeInsets.all(4),
      child: MouseRegion(
        onEnter: _onEnter,
        onExit: _onExit,
        child: AnimatedContainer(
          constraints: const BoxConstraints(minHeight: 56),
          width: double.infinity,
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
          duration: theme.fastAnimationDuration,
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(widget.radius),
            color: _isHovered
                ? theme.resources.controlFillColorSecondary
                : theme.resources.controlFillColorDefault,
          ),
          child: ClipRRect(
            borderRadius: BorderRadius.circular(widget.radius - 1),
            child: SmallerOrEqualToScreenSize(
              maxSize: 200,
              builder: (context, isMini) {
                return Row(
                  mainAxisAlignment: MainAxisAlignment.spaceBetween,
                  children: [
                    if (!isMini && widget.icon != null)
                      Container(
                        width: 36,
                        height: 36,
                        decoration: BoxDecoration(
                          color: widget.iconColor ?? theme.accentColor,
                          borderRadius: BorderRadius.circular(2),
                        ),
                        child: Icon(
                          widget.icon,
                          color: theme.activeColor,
                          size: 26,
                        ),
                      ),
                    if (!isMini && widget.icon != null)
                      const SizedBox(width: 12),
                    if (isMini && widget.icon != null)
                      const SizedBox(height: 48, width: 4),
                    Expanded(
                      child: SettingsBlockTitle(
                        title: widget.title,
                        subtitle: widget.subtitle,
                      ),
                    ),
                    const SizedBox(width: 8),
                    widget.child,
                  ],
                );
              },
            ),
          ),
        ),
      ),
    );
  }
}
