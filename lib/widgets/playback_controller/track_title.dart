import 'package:fluent_ui/fluent_ui.dart';

class TrackTitle extends StatefulWidget {
  const TrackTitle({
    super.key,
    required this.title,
    required this.style,
    required this.onPressed,
    this.focusNode,
    this.autofocus = false,
    this.focusable = true,
  });

  final String title;
  final TextStyle? style;
  final VoidCallback? onPressed;
  final FocusNode? focusNode;
  final bool autofocus;
  final bool focusable;

  @override
  State<TrackTitle> createState() => _TrackTitleState();
}

class _TrackTitleState extends State<TrackTitle> {
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

    final textColorBase =
        _isFocused ? theme.accentColor : theme.resources.textFillColorPrimary;
    final textColor = _isHovered ? textColorBase.withAlpha(160) : textColorBase;

    List<Shadow>? textShadow;

    if (_isFocused) {
      textShadow = [
        Shadow(
          color: theme.accentColor.withValues(alpha: 0.5),
          blurRadius: 4,
        ),
      ];
    } else {
      textShadow = [
        Shadow(
          color: Colors.transparent,
          blurRadius: 0,
        ),
      ];
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
        child: Text(
          widget.title,
          overflow: TextOverflow.ellipsis,
          style: widget.style
              ?.merge(TextStyle(color: textColor, shadows: textShadow)),
        ),
      ),
    );
  }
}
