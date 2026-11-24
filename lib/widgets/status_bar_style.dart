import 'package:flutter/services.dart';
import 'package:fluent_ui/fluent_ui.dart';

/// Makes the system status bar text readable on the current theme. Only the
/// status bar is set, so the navigation bar of Android stays as it was.
class StatusBarStyle extends StatelessWidget {
  const StatusBarStyle({super.key, this.brightness, required this.child});

  /// The brightness of what is under the status bar, for a page whose
  /// background does not follow the theme. The theme's when null.
  final Brightness? brightness;

  final Widget child;

  static const _onDark = SystemUiOverlayStyle(
    statusBarBrightness: Brightness.dark,
    statusBarIconBrightness: Brightness.light,
  );
  static const _onLight = SystemUiOverlayStyle(
    statusBarBrightness: Brightness.light,
    statusBarIconBrightness: Brightness.dark,
  );

  @override
  Widget build(BuildContext context) {
    final dark = (brightness ?? FluentTheme.of(context).brightness).isDark;

    return AnnotatedRegion<SystemUiOverlayStyle>(
      value: dark ? _onDark : _onLight,
      child: child,
    );
  }
}
