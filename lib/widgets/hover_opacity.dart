import 'package:fluent_ui/fluent_ui.dart';

class HoverOpacity extends StatefulWidget {
  final Widget child;
  final double hoverOpacity;
  final double normalOpacity;

  const HoverOpacity({
    super.key,
    required this.child,
    this.hoverOpacity = 1.0,
    this.normalOpacity = 0.7,
  });

  @override
  HoverOpacityState createState() => HoverOpacityState();
}

class HoverOpacityState extends State<HoverOpacity> {
  bool _isHovered = false;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return MouseRegion(
      onEnter: (_) => setState(() => _isHovered = true),
      onExit: (_) => setState(() => _isHovered = false),
      child: AnimatedOpacity(
        opacity: _isHovered ? widget.hoverOpacity : widget.normalOpacity,
        duration: theme.fastAnimationDuration,
        child: widget.child,
      ),
    );
  }
}
