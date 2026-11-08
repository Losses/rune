import 'package:flutter_test/flutter_test.dart';
import '../l10n/app_localizations_en.dart';
import '../providers/library_path.dart';
import '../utils/library_reference.dart';
import '../utils/library_ui/library_ui_state.dart';

void main() {
  const id = '6f1c2b1e-0d5a-4d0e-9b9e-2f6a3e8c1a11';
  const replica = 'replica://$id';

  test('only the replica scheme marks a replica', () {
    expect(isReplicaReference(replica), isTrue);
    expect(isReplicaReference('/Users/me/Music'), isFalse);
    expect(isReplicaReference('@RR|replica://$id'), isFalse);
  });

  test('a replica row is titled by its library id', () {
    expect(libraryFolderName(replica), id);
    expect(libraryFolderName('/Users/me/Music'), 'Music');
  });

  test('a replica name gives the id as alias and stays local', () {
    final entry = LibraryPathEntry(replica);
    expect(entry.alias, id);
    expect(LibraryPathEntry.removePrefix(replica), replica);
  });

  test('a replica name is not shown as an iOS Files location', () {
    expect(
      filesAppLocation(replica, onThisDevice: 'Device', iCloudDrive: 'iCloud'),
      replica,
    );
    expect(describeLibraryPath(AppLocalizationsEn(), replica), replica);
  });
}
