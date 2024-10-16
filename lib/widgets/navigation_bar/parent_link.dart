import 'package:fluent_ui/fluent_ui.dart';

import 'flip_text.dart';

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
  double _glowRadius = 0;

  void _handleFocusHighlight(bool value) {
    setState(() {
      _glowRadius = value ? 20 : 0;
    });
  }

  void _handleHoveHighlight(bool value) {
    setState(() {
      _alpha = value ? 100 : 80;
    });
  }

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(right: 12),
      child: GestureDetector(
        onTap: widget.onTap,
        child: FocusableActionDetector(
          onShowFocusHighlight: _handleFocusHighlight,
          onShowHoverHighlight: _handleHoveHighlight,
          child: SizedBox(
            height: 80,
            width: 320,
            child: FlipText(
              key: Key(widget.titleFlipKey),
              flipKey: widget.titleFlipKey,
              text: widget.text,
              scale: 6,
              alpha: _alpha,
              glowColor: Colors.red,
              glowRadius: _glowRadius,
            ),
          ),
        ),
      ),
    );
  }
}
