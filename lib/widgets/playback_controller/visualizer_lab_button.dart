import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../providers/visualizer_lab.dart';
import '../../providers/router_path.dart';

import '../rune_clickable.dart';

import 'cover_wall_button.dart';

/// Toggles the lab over the visualizer on the cover wall page, which opens
/// first if it is not on screen.
void toggleVisualizerLab(BuildContext context) {
  final lab = Provider.of<VisualizerLabProvider>(context, listen: false);
  if ($router.path == kLabPath) {
    lab.toggleOverlay();
  } else {
    showCoverArtWall();
    lab.setOverlayOpen(true);
  }
}

class VisualizerLabButton extends StatelessWidget {
  final List<Shadow>? shadows;

  const VisualizerLabButton({super.key, required this.shadows});

  @override
  Widget build(BuildContext context) {
    return RuneClickable(
      onPressed: () => toggleVisualizerLab(context),
      child: Icon(Symbols.science, shadows: shadows),
    );
  }
}
