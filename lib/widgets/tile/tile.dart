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

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    Color borderColor;
    List<BoxShadow>? boxShadow;

    if (_isHovered) {
      borderColor = theme.resources.controlStrokeColorDefault;
    } else if (_isFocused) {
      borderColor = theme.accentColor;
      boxShadow = [
        BoxShadow(
          color: theme.accentColor.withOpacity(0.5),
          blurRadius: 10,
          spreadRadius: 2,
        ),
      ];
    } else {
      borderColor = theme.resources.controlStrokeColorSecondary;
    }

    return GestureDetector(
      onTap: widget.onPressed,
      child: FocusableActionDetector(
        onShowFocusHighlight: _handleFocusHighlight,
        onShowHoverHighlight: _handleHoverHighlight,
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
            boxShadow: _isFocused ? boxShadow : null,
          ),
          child: ClipRRect(
            borderRadius: BorderRadius.circular(widget.radius - 1),
            child: widget.child,
          ),
        ),
      ),
    );
  }
}
