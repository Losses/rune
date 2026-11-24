import 'package:fluent_ui/fluent_ui.dart';

/// Esc for Rune's own navigation: leave full screen, search, lyrics or the
/// cover wall.
///
/// Not a [DismissIntent]: every page route maps that intent to its own action,
/// which is disabled unless the route can be dismissed, and the shortcut
/// manager stops at the nearest one, so Esc never reached Rune's action.
class EscapeIntent extends Intent {
  const EscapeIntent();
}
