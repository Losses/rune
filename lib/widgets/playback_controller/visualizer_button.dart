import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../providers/router_path.dart';
import '../../utils/router/navigation.dart';
import '../../utils/navigation/utils/escape_from_visualizer.dart';

import '../rune_clickable.dart';

void showVisualizer() {
  final path = $router.path;
  if (path == '/visualizer') {
    escapeFromVisualizer();
  } else {
    if (path == '/cover_wall' || path == '/lyrics') {
      $replace('/visualizer');
    } else {
      $push('/visualizer');
    }
  }
}

class VisualizerButton extends StatelessWidget {
  final List<Shadow>? shadows;

  const VisualizerButton({
    super.key,
    required this.shadows,
  });

  @override
  Widget build(BuildContext context) {
    return RuneClickable(
      onPressed: () => showVisualizer(),
      child: Icon(
        Symbols.graphic_eq,
        shadows: shadows,
      ),
    );
  }
}
