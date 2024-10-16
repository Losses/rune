import 'package:fluent_ui/fluent_ui.dart';

class Tile extends StatefulWidget {
  const Tile({
    super.key,
    required this.onPressed,
    required this.child,
    this.radius = 4,
    this.borderWidth,
  });

  final VoidCallback? onPressed;
  final Widget child;
  final double radius;
  final double? borderWidth;

  @override
  TileState createState() => TileState();
}

class TileState extends State<Tile> {
  bool _isHovered = false;
  final FocusNode _focusNode = FocusNode();

  @override
  void initState() {
    super.initState();
    _focusNode.addListener(() {
      setState(() {});
    });
  }

  @override
  void dispose() {
    _focusNode.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    Color borderColor;

    if (_isHovered || _focusNode.hasFocus) {
      borderColor = theme.resources.controlStrokeColorDefault;
    } else {
      borderColor = theme.resources.controlStrokeColorSecondary;
    }

    return GestureDetector(
      onTap: widget.onPressed,
      child: MouseRegion(
        onEnter: (_) => setState(() => _isHovered = true),
        onExit: (_) => setState(() => _isHovered = false),
        child: FocusableActionDetector(
          focusNode: _focusNode,
          child: AnimatedContainer(
            duration: theme.fastAnimationDuration,
            width: double.infinity,
            height: double.infinity,
            decoration: BoxDecoration(
              border: Border.all(
                color: borderColor,
                width: widget.borderWidth ?? 1,
              ),
              borderRadius: BorderRadius.circular(widget.radius),
            ),
            child: ClipRRect(
              borderRadius: BorderRadius.circular(widget.radius - 1),
              child: widget.child,
            ),
          ),
        ),
      ),
    );
  }
}
