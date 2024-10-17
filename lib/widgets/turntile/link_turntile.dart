import 'package:go_router/go_router.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/widgets/navigation_bar/utils/activate_link_action.dart';

import '../../widgets/ax_pressure.dart';

class LinkTurntile extends StatefulWidget {
  final String title;
  final String path;
  final IconData? icon;

  const LinkTurntile({
    super.key,
    required this.title,
    required this.path,
    this.icon,
  });

  @override
  State<LinkTurntile> createState() => _LinkTurntileState();
}

class _LinkTurntileState extends State<LinkTurntile> {
  bool _isHovered = false;
  bool _isFocused = false;

  void _handleFocusHighlight(bool value) {
    setState(() {
      _isFocused = value;
    });
  }

  void _handleHoverHighlight(bool value) {
    setState(() {
      _isHovered = value;
    });
  }

  void onPressed() {
    context.push(widget.path);
  }

  @override
  void dispose() {
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    final color = (_isHovered || _isFocused)
        ? theme.accentColor.withAlpha(255)
        : theme.typography.title!.color!.withAlpha(180);

    return AxPressure(
      child: GestureDetector(
        onTap: onPressed,
        child: FocusableActionDetector(
          onShowFocusHighlight: _handleFocusHighlight,
          onShowHoverHighlight: _handleHoverHighlight,
          actions: {
            ActivateIntent: ActivateLinkAction(context, onPressed),
          },
          child: TweenAnimationBuilder(
            tween: ColorTween(
              begin: color,
              end: color,
            ),
            duration: theme.fastAnimationDuration,
            builder: (BuildContext context, Color? color, Widget? child) {
              return TweenAnimationBuilder(
                tween: Tween<double>(
                  begin: 0.0,
                  end: 0.0,
                ),
                duration: theme.fastAnimationDuration,
                builder:
                    (BuildContext context, double blurRadius, Widget? child) {
                  return Text(
                    widget.title,
                    textAlign: TextAlign.start,
                    style: theme.typography.title?.apply(
                      fontWeightDelta: -100,
                      color: color,
                      shadows: [
                        Shadow(
                          color: theme.accentColor,
                          blurRadius: blurRadius,
                        ),
                      ],
                    ),
                  );
                },
              );
            },
          ),
        ),
      ),
    );
  }
}
