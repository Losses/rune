/// The argument that launches the app with text box cursors that blink on a
/// timer instead of fading.
///
/// A fading cursor asks for a frame at every vsync while a box has focus,
/// which keeps a small board drawing empty frames.
const blinkCursorArgument = '--blink-cursor';

/// The environment variable that does the same when it is 1, for embedders
/// that pass no arguments to the app: flutter-pi starts the Dart entrypoint
/// without any.
const blinkCursorVariable = 'RUNE_BLINK_CURSOR';

/// Whether the app was launched with blinking text box cursors.
bool cursorBlinksAtLaunch(
  List<String> arguments,
  Map<String, String> environment,
) =>
    arguments.contains(blinkCursorArgument) ||
    environment[blinkCursorVariable] == '1';
