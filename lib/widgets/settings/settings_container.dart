import 'package:fluent_ui/fluent_ui.dart';

class SettingsContainer extends StatefulWidget {
  const SettingsContainer({
    super.key,
    required this.child,
    this.radius = 4,
    this.margin = const EdgeInsets.all(4),
    this.padding = const EdgeInsets.all(0),
  });

  final Widget child;
  final double radius;
  final EdgeInsetsGeometry margin;
  final EdgeInsetsGeometry padding;

  @override
  SettingsBlockState createState() => SettingsBlockState();
}

class SettingsBlockState extends State<SettingsContainer> {
  bool _isHovered = false;

  final FocusNode _focusNode = FocusNode(debugLabel: 'Settings Block');

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
      padding: widget.margin,
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
            child: Padding(
              padding: widget.padding,
              child: widget.child,
            ),
          ),
        ),
      ),
    );
  }
}
