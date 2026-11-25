import 'package:flutter_test/flutter_test.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations_en.dart';
import 'package:rune/utils/library_registry_messages.dart';

void main() {
  test('a library from a newer Rune asks for Rune to be updated', () {
    final s = AppLocalizationsEn();
    expect(
      libraryRegistryFailureMessage(s, OpenLibraryFailureKind.newerVersion),
      s.libraryNewerVersion,
    );
    // Storage failures keep their own message.
    expect(
      libraryRegistryFailureMessage(s, OpenLibraryFailureKind.storage),
      s.libraryStorageFailure,
    );
  });

  test('a replica without sync access says the library is not shared', () {
    final s = AppLocalizationsEn();
    expect(
      libraryRegistryFailureMessage(s, RegisterLibraryFailureKind.syncNotShared),
      s.libraryNotShared,
    );
  });
}
