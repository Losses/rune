import 'package:fluent_ui/fluent_ui.dart';

class RuneClickable extends StatefulWidget {
  const RuneClickable({
    super.key,
    required this.child,
    required this.onPressed,
    this.focusNode,
    this.autofocus = false,
    this.focusable = true,
    this.padding = 8.0,
    this.iconSize = 16.0,
    this.isTiny = false,
  });

  final Widget child;
  final VoidCallback? onPressed;
  final FocusNode? focusNode;
  final bool autofocus;
  final bool focusable;
  final double padding;
  final double iconSize;
  final bool isTiny;

  @override
  State<RuneClickable> createState() => _RuneClickableState();
}

class _RuneClickableState extends State<RuneClickable> {
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
    final iconColor = widget.isTiny
        ? _isHovered
            ? theme.resources.textFillColorPrimary.withAlpha(160)
            : theme.resources.textFillColorPrimary
        : theme.resources.textFillColorPrimary;

    final fillColor = widget.isTiny
        ? Colors.transparent
        : _isFocused
            ? theme.resources.solidBackgroundFillColorBase
            : _isHovered
                ? theme.resources.subtleFillColorSecondary
                : Colors.transparent;

    Color borderColor;
    List<BoxShadow>? boxShadow;
    double borderWidth = 0;

    if (_isFocused) {
      borderColor = theme.accentColor;
      boxShadow = [
        BoxShadow(
          color: theme.accentColor.withValues(alpha: 0.5),
          blurRadius: 10,
          spreadRadius: 2,
        ),
      ];
      borderWidth = 2;
    } else if (_isHovered) {
      borderColor = theme.resources.controlStrokeColorDefault;
    } else {
      borderColor = Colors.transparent;
    }

    return FocusableActionDetector(
      focusNode: widget.focusNode,
      autofocus: widget.autofocus,
      onShowFocusHighlight: _handleFocusHighlight,
      onShowHoverHighlight: _handleHoverHighlight,
      actions: {
        ActivateIntent:
            CallbackAction(onInvoke: (e) => widget.onPressed?.call()),
      },
      child: Listener(
        onPointerUp: (_) => widget.onPressed?.call(),
        child: AnimatedContainer(
          duration: const Duration(milliseconds: 200),
          padding: EdgeInsets.all(widget.padding),
          decoration: BoxDecoration(
            color: fillColor,
            border: Border.all(
              color: borderColor,
              width: borderWidth,
            ),
            borderRadius: BorderRadius.circular(4.0),
            boxShadow: _isFocused ? boxShadow : null,
          ),
          child: IconTheme(
            data: IconThemeData(
              size: widget.iconSize,
              color: iconColor,
            ),
            child: widget.child,
          ),
        ),
      ),
    );
  }
}
