import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/navigation/navigation_item.dart';

import 'flip_text.dart';
import 'utils/activate_link_action.dart';

class SlibingLink extends StatefulWidget {
  final NavigationItem route;
  final bool isSelected;
  final int? delay;
  final VoidCallback onTap;

  const SlibingLink({
    super.key,
    required this.route,
    required this.isSelected,
    required this.delay,
    required this.onTap,
  });

  @override
  State<SlibingLink> createState() => _SlibingLinkState();
}

class _SlibingLinkState extends State<SlibingLink> {
  Timer? timer;
  late double _entryAnimationOpacity;

  bool _isHovered = false;
  bool _isFocus = false;

  @override
  void initState() {
    super.initState();
    final delay = widget.delay;
    if (delay != null) {
      timer = Timer(Duration(milliseconds: delay), () {
        if (!mounted) return;

        setState(() {
          _entryAnimationOpacity = 1;
        });
      });
      _entryAnimationOpacity = 0;
    } else {
      _entryAnimationOpacity = 1;
    }
  }

  @override
  void dispose() {
    super.dispose();

    timer?.cancel();
  }

  void _handleFocusHighlight(bool value) {
    setState(() {
      _isFocus = value;
    });
  }

  void _handleHoveHighlight(bool value) {
    setState(() {
      _isHovered = value;
    });
  }

  @override
  Widget build(BuildContext context) {
    final accentColor = FluentTheme.of(context).accentColor;
    final childFlipKey = 'child:${widget.route.path}';

    return Padding(
      padding: const EdgeInsets.only(right: 24),
      child: GestureDetector(
        onTap: widget.onTap,
        child: FocusableActionDetector(
          onShowFocusHighlight: _handleFocusHighlight,
          onShowHoverHighlight: _handleHoveHighlight,
          actions: {
            ActivateIntent: ActivateLinkAction(context, widget.onTap),
          },
          child: AnimatedOpacity(
            key: Key('animation-$childFlipKey'),
            opacity: _entryAnimationOpacity,
            duration: const Duration(milliseconds: 300),
            child: FlipText(
              key: Key(childFlipKey),
              flipKey: childFlipKey,
              text: widget.route.title,
              scale: 1.2,
              color: _isFocus ? accentColor : null,
              glowColor: accentColor,
              glowRadius: _isFocus ? 10 : 0,
              alpha: widget.isSelected || _isFocus
                  ? 255
                  : _isHovered
                      ? 200
                      : 100,
            ),
          ),
        ),
      ),
    );
  }
}
