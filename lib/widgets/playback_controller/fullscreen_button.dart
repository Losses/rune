import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:provider/provider.dart';

import '../../providers/full_screen.dart';

import '../rune_icon_button.dart';

class FullScreenButton extends StatelessWidget {
  final List<Shadow>? shadows;
  const FullScreenButton({
    super.key,
    required this.shadows,
  });

  @override
  Widget build(BuildContext context) {
    final fullScreen = Provider.of<FullScreenProvider>(context);

    return RuneIconButton(
      onPressed: () => fullScreen.setFullScreen(!fullScreen.isFullScreen),
      icon: fullScreen.isFullScreen
          ? Icon(
              Symbols.fullscreen_exit,
              shadows: shadows,
            )
          : Icon(
              Symbols.fullscreen,
              shadows: shadows,
            ),
    );
  }
}
