import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import 'package:player/widgets/tiny_icon_button.dart';

class LikeButton extends StatelessWidget {
  final bool disabled;

  final bool liked;

  const LikeButton({required this.disabled, required this.liked, super.key});

  @override
  Widget build(BuildContext context) {
    return TinyIconButton(
      onPressed: disabled
          ? null
          : () {
              if (liked) {
                //
              } else {}
            },
      icon: Icon(
        Symbols.favorite,
        fill: liked ? 1 : 0,
      ),
    );
  }
}
