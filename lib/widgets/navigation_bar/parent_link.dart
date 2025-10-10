import 'package:fluent_ui/fluent_ui.dart';

import 'flip_text.dart';
import 'utils/activate_link_action.dart';

class ParentLink extends StatefulWidget {
  final String titleFlipKey;
  final String text;
  final VoidCallback onPressed;

  const ParentLink({
    super.key,
    required this.titleFlipKey,
    required this.text,
    required this.onPressed,
  });

  @override
  ParentLinkState createState() => ParentLinkState();
}

class ParentLinkState extends State<ParentLink> {
  double _alpha = 80;
  bool _isFocus = false;
  double? _cachedScaledWidth;
  String? _cachedText;

  final FocusNode _focusNode = FocusNode(debugLabel: 'Parent Link');

  @override
  void dispose() {
    super.dispose();
    _focusNode.dispose();
  }

  void _handleFocusHighlight(bool value) {
    setState(() {
      _isFocus = value;
    });
  }

  void _handleHoverHighlight(bool value) {
    setState(() {
      _alpha = value ? 100 : 80;
    });
  }

  double _getScaledWidth() {
    if (_cachedScaledWidth == null || _cachedText != widget.text) {
      final textPainter = TextPainter(
        text: TextSpan(
          text: widget.text,
          style: const TextStyle(
            fontSize: 17,
            fontVariations: <FontVariation>[FontVariation('wght', 400)],
          ),
        ),
        textDirection: TextDirection.ltr,
      );
      textPainter.layout();

      _cachedScaledWidth = (textPainter.width * 5) + 12;
      _cachedText = widget.text;
    }

    return _cachedScaledWidth!;
  }

  void _onPressed() {
    if (comboBoxOpened) {
      return;
    }

    widget.onPressed();
  }

  @override
  Widget build(BuildContext context) {
    final accentColor = FluentTheme.of(context).accentColor;

    return Container(
      constraints: BoxConstraints.tightFor(width: _getScaledWidth()),
      padding: const EdgeInsets.only(right: 12),
      child: Listener(
        onPointerUp: (_) => _onPressed(),
        child: FocusableActionDetector(
          focusNode: _focusNode,
          onShowFocusHighlight: _handleFocusHighlight,
          onShowHoverHighlight: _handleHoverHighlight,
          actions: {ActivateIntent: ActivateLinkAction(context, _onPressed)},
          child: SizedBox(
            height: 80,
            child: FlipText(
              key: Key(widget.titleFlipKey),
              flipKey: widget.titleFlipKey,
              text: widget.text,
              scale: 5,
              alpha: _isFocus ? 255 : _alpha,
              color: _isFocus ? accentColor : null,
              glowColor: accentColor,
              glowRadius: _isFocus ? 10 : 0,
            ),
          ),
        ),
      ),
    );
  }
}
