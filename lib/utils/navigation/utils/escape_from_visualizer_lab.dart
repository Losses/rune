import 'package:provider/provider.dart';

import '../../../providers/responsive_providers.dart';
import '../../../providers/visualizer_lab.dart';

import '../../router/navigation.dart';

/// Closes the lab mode, if it is on. Whether it is is up to the provider's
/// state and the route. A lab the layout has no room for is not on screen,
/// so it leaves the key to the rest.
bool escapeFromVisualizerLab() {
  final context = $context();
  final lab = Provider.of<VisualizerLabProvider>(context, listen: false);
  if (!lab.overlayOpen) return false;
  final responsive = Provider.of<ResponsiveProvider>(context, listen: false);
  if (!visualizerLabFits(responsive)) return false;
  lab.setOverlayOpen(false);
  return true;
}
