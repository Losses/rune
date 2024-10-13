import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/playback_controller/constants/controller_items.dart';

class ControllerIntent extends Intent {
  final ControllerEntry entry;
  const ControllerIntent(this.entry);
}
