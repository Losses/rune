import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/navigation/navigation_item.dart';

import './flip_text.dart';

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
  double _glowRadius = 0;

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
      _glowRadius = value ? 20 : 0;
    });
  }

  void _handleHoveHighlight(bool value) {
    setState(() {
      _isHovered = value;
    });
  }

  @override
  Widget build(BuildContext context) {
    final childFlipKey = 'child:${widget.route.path}';

    return Padding(
      padding: const EdgeInsets.only(right: 24),
      child: GestureDetector(
        onTap: widget.onTap,
        child: FocusableActionDetector(
          onShowFocusHighlight: _handleFocusHighlight,
          onShowHoverHighlight: _handleHoveHighlight,
          child: AnimatedOpacity(
            key: Key('animation-$childFlipKey'),
            opacity: _entryAnimationOpacity,
            duration: const Duration(milliseconds: 300),
            child: FlipText(
              key: Key(childFlipKey),
              flipKey: childFlipKey,
              text: widget.route.title,
              scale: 1.2,
              glowColor: Colors.red,
              glowRadius: _glowRadius,
              alpha: widget.isSelected
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

