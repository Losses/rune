import 'package:fluent_ui/fluent_ui.dart';

class WindowIconButton extends StatefulWidget {
  final VoidCallback onPressed;
  final Widget? child;

  const WindowIconButton({
    super.key,
    required this.onPressed,
    this.child,
  });

  @override
  State<WindowIconButton> createState() => _WindowIconButtonState();
}

class _WindowIconButtonState extends State<WindowIconButton> {
  bool isHovered = false;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return MouseRegion(
      onEnter: (_) => setState(() => isHovered = true),
      onExit: (_) => setState(() => isHovered = false),
      child: Listener(
        onPointerUp: (_) => widget.onPressed(),
        child: Container(
          width: 46,
          height: 30,
          decoration: BoxDecoration(
            color: isHovered
                ? theme.resources.textFillColorPrimary.withOpacity(0.08)
                : Colors.transparent,
          ),
          child: widget.child,
        ),
      ),
    );
  }
}
