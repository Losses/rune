import 'package:fluent_ui/fluent_ui.dart';
import 'package:player/config/navigation.dart';

import '../utils/navigation/build_shortcuts.dart';

final Map<LogicalKeySet, Intent> shortcuts = buildShortcuts(navigationItems);
