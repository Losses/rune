import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/navigation_bar/utils/activate_link_action.dart';

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

  Timer? _delayTimer;

  final FocusNode _focusNode = FocusNode(debugLabel: 'Tile');

  @override
  void dispose() {
    super.dispose();
    _focusNode.dispose();
  }

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

  void _handleDelayedPress(PointerUpEvent _) {
    if (widget.onPressed == null) return;

    // Cancel any existing timer
    _delayTimer?.cancel();

    _delayTimer = Timer(const Duration(milliseconds: 120), () {
      if (mounted) {
        widget.onPressed?.call();
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    Color borderColor;
    List<BoxShadow>? boxShadow;
    double borderWidth = widget.borderWidth ?? 1;

    if (_isFocused) {
      borderColor = theme.accentColor;
      boxShadow = [
        BoxShadow(
          color: theme.accentColor.withValues(alpha: 0.5),
          blurRadius: 10,
          spreadRadius: 2,
        ),
      ];
      borderWidth *= 2;
    } else if (_isHovered) {
      borderColor = theme.resources.controlStrokeColorDefault;
    } else {
      borderColor = theme.resources.controlStrokeColorSecondary;
    }

    return Listener(
      onPointerUp: _handleDelayedPress,
      child: FocusableActionDetector(
        focusNode: _focusNode,
        onShowFocusHighlight: _handleFocusHighlight,
        onShowHoverHighlight: _handleHoverHighlight,
        actions: {
          ActivateIntent: ActivateLinkAction(context, widget.onPressed),
        },
        child: AnimatedContainer(
          duration: theme.fastAnimationDuration,
          width: double.infinity,
          height: double.infinity,
          decoration: BoxDecoration(
            border: Border.all(
              color: borderColor,
              width: borderWidth,
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
