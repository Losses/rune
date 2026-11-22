import '../providers/library_registry.dart';

/// Leaves the library route for the library chooser when the library Rune
/// reopens at startup does not open. The app starts on that route before the
/// open has finished, and the route shows nothing without an open library.
///
/// When the server rejected the saved password, [signInAgain] asks for new
/// credentials, opens the library with them and returns whether it did; it
/// returns false when the user cancels. [showChooser] gets the reason to tell
/// the user: the failure of the last attempt, or null when it is not known.
Future<void> recoverFailedStartupOpen({
  required Future<bool> libraryOpened,
  required Object? Function() failure,
  required bool Function() libraryIsOpen,
  required Future<bool> Function() signInAgain,
  required void Function(Object? failure) showChooser,
}) async {
  if (await libraryOpened || libraryIsOpen()) return;
  var reason = failure();
  if (needsLibraryRelogin(reason)) {
    try {
      if (await signInAgain()) return;
    } on Object catch (error) {
      reason = error;
    }
  }
  if (!libraryIsOpen()) showChooser(reason);
}
