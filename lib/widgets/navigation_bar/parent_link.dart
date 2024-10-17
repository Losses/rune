import 'package:fluent_ui/fluent_ui.dart';

import 'flip_text.dart';
import 'utils/activate_link_action.dart';

class ParentLink extends StatefulWidget {
  final String titleFlipKey;
  final String text;
  final VoidCallback onTap;

  const ParentLink({
    super.key,
    required this.titleFlipKey,
    required this.text,
    required this.onTap,
  });

  @override
  ParentLinkState createState() => ParentLinkState();
}

class ParentLinkState extends State<ParentLink> {
  double _alpha = 80;
  bool _isFocus = false;

  void _handleFocusHighlight(bool value) {
    setState(() {
      _isFocus = value;
    });
  }

  void _handleHoveHighlight(bool value) {
    setState(() {
      _alpha = value ? 100 : 80;
    });
  }

  @override
  Widget build(BuildContext context) {
    final accentColor = FluentTheme.of(context).accentColor;

    return Padding(
      padding: const EdgeInsets.only(right: 12),
      child: GestureDetector(
        onTap: widget.onTap,
        child: FocusableActionDetector(
          onShowFocusHighlight: _handleFocusHighlight,
          onShowHoverHighlight: _handleHoveHighlight,
          actions: {
            ActivateIntent: ActivateLinkAction(context, widget.onTap),
          },
          child: SizedBox(
            height: 80,
            width: 320,
            child: FlipText(
              key: Key(widget.titleFlipKey),
              flipKey: widget.titleFlipKey,
              text: widget.text,
              scale: 6,
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
