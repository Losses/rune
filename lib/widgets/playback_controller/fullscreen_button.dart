import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:provider/provider.dart';

import '../../providers/full_screen.dart';

class FullScreenButton extends StatelessWidget {
  const FullScreenButton({super.key});

  @override
  Widget build(BuildContext context) {
    final fullScreen = Provider.of<FullScreenProvider>(context);

    return IconButton(
      onPressed: () => fullScreen.setFullScreen(!fullScreen.isFullScreen),
      icon: fullScreen.isFullScreen
          ? const Icon(Symbols.fullscreen_exit)
          : const Icon(Symbols.fullscreen),
    );
  }
}
