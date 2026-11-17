import 'package:flutter/services.dart';

/// The environment variable that sets how many seconds without input switch the
/// display off, on an embedder that supports it (the flutter-pi of the embedded
/// kiosk). 0 keeps the display on. Left out, the app does not touch the
/// display's power.
///
/// The embedder passes no arguments to the app, so the value comes in this
/// variable, as it does for `blinkCursorVariable`.
const blankAfterVariable = 'RUNE_BLANK_AFTER';

/// The seconds without input after which the display goes off, as the launch
/// environment gives them, or null when it gives none or something that is not
/// a number of seconds.
int? blankAfterAtLaunch(Map<String, String> environment) {
  final value = int.tryParse(environment[blankAfterVariable] ?? '');
  if (value == null || value < 0) return null;
  return value;
}

/// Switches the display off after a time without input, and back on with a
/// double tap, through an embedder that offers it.
///
/// While the display is off the embedder tells Flutter the app is paused (see
/// `AppLifecycleState`), so no frames are drawn, and drops every touch except
/// the double tap that wakes the display. Other embedders do not answer, and
/// the display stays on.
class DisplayBlanker {
  const DisplayBlanker._();

  static const MethodChannel _channel = MethodChannel('rune/display_power');

  /// Sets the seconds without input after which the display goes off. 0 keeps
  /// it on. Returns whether an embedder took the setting; one that is missing
  /// or refuses the value leaves the display on, and does not throw.
  static Future<bool> setBlankAfter(int seconds) async {
    try {
      await _channel.invokeMethod<void>('setBlankAfter', seconds);
      return true;
    } on MissingPluginException {
      return false;
    } on PlatformException {
      // The embedder refused the value, such as a time too long for it.
      return false;
    }
  }
}
