import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';

import 'rune_icon_button.dart';

class BandScreenFallbackButton extends StatelessWidget {
  final VoidCallback onPressed;
  final Widget icon;

  const BandScreenFallbackButton({
    super.key,
    required this.icon,
    required this.onPressed,
  });

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraint) {
        final size = min(constraint.maxWidth, constraint.maxHeight);

        return RuneIconButton(
          icon: icon,
          iconSize: (size * 0.8).clamp(0, 48),
          padding: (size * 0.1).clamp(0, 12),
          onPressed: () => {},
        );
      },
    );
  }
}
