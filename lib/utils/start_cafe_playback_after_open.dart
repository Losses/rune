import 'rune_log.dart';

/// Plays once the library has opened. A failure is logged and goes no further,
/// as nothing awaits this and the library may not answer what [play] asks.
Future<void> startCafePlaybackAfterOpen({
  required Future<bool> libraryOpened,
  required Future<void> Function() play,
}) async {
  if (!await libraryOpened) return;
  try {
    await play();
  } catch (error) {
    error$('Cafe playback failed: $error');
  }
}
