import 'package:fluent_ui/fluent_ui.dart';

import '../utils/navigation/build_shortcuts.dart';

final Map<SingleActivator, Intent> shortcuts = buildShortcuts();
final Map<SingleActivator, Intent> noShortcuts = buildNoShortcuts(shortcuts);