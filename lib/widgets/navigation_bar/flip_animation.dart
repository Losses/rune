import 'package:fluent_ui/fluent_ui.dart';

import './flip_animation_manager.dart';

class FlipAnimationContext extends StatelessWidget {
  final Widget child;

  const FlipAnimationContext({super.key, required this.child});

  @override
  Widget build(BuildContext context) {
    return Stack(
      alignment: Alignment.center,
      children: [
        SizedBox.expand(
          child: FlipAnimationManager(child: child),
        ),
        const Overlay(
          initialEntries: [],
        ),
      ],
    );
  }
}
