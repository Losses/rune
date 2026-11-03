import 'package:flutter_test/flutter_test.dart';
import '../bindings/bindings.dart';
import '../l10n/app_localizations_en.dart';
import '../utils/library_ui/library_ui_state.dart';

void main() {
  final s = AppLocalizationsEn();
  test('peer responses map loading, error, empty and added rows', () {
    expect(peerLibrariesState(s, null).status, PeerLibrariesStatus.loading);
    final error = peerLibrariesState(
      s,
      const ListPeerLibrariesResponse(
        libraries: [],
        errorKind: LibraryOpenErrorKind.remoteUnreachable,
      ),
    );
    expect(error.status, PeerLibrariesStatus.error);
    expect(error.error, s.remoteOpenUnreachable);
    expect(
      peerLibrariesState(
        s,
        const ListPeerLibrariesResponse(libraries: []),
      ).status,
      PeerLibrariesStatus.empty,
    );
    const libraries = [
      PeerLibrary(
        libraryId: '1', alias: 'One', added: false,
        accessLevel: DeviceLibraryLevel.read,
      ),
      PeerLibrary(
        libraryId: '2', alias: 'Two', added: true,
        accessLevel: DeviceLibraryLevel.readWrite,
      ),
    ];
    final rows = peerLibrariesState(
      s,
      const ListPeerLibrariesResponse(libraries: libraries),
    );
    expect(rows.status, PeerLibrariesStatus.rows);
    expect(rows.libraries.map((e) => e.added), [false, true]);
  });
  test('registry rows match path and choose device alias and switch', () {
    const libraries = [
      LocalLibrary(
        libraryId: '1',
        alias: 'Music',
        path: '/one',
        ownerFingerprint: 'peer',
        backgroundSync: true,
        accessLevel: DeviceLibraryLevel.read,
      ),
      LocalLibrary(
        libraryId: '2',
        alias: 'Local',
        path: '/two',
        backgroundSync: false,
      ),
    ];
    LibraryRowState row(
      String path, [
      Map<String, String> aliases = const {},
    ]) => libraryRowState(
      path, libraries, aliases, (device) => 'From $device', s.readOnly,
    );
    expect(row('/one', {'peer': 'Laptop'}).subtitle, 'From Laptop · Read only');
    expect(row('/one').subtitle, 'From Music · Read only');
    expect(row('/one').library?.backgroundSync, true);
    expect(row('/two').subtitle, '/two');
    expect(row('/two').library?.backgroundSync, false);
    expect(row('/missing').library, isNull);
  });
  test('only the current read replica is read only', () {
    LocalLibrary library(String path, String? owner, DeviceLibraryLevel? level) =>
        LocalLibrary(
          libraryId: path,
          alias: path,
          path: path,
          ownerFingerprint: owner,
          backgroundSync: true,
          accessLevel: level,
        );
    final libraries = [
      library('/owned', null, DeviceLibraryLevel.read),
      library('/read', 'peer', DeviceLibraryLevel.read),
      library('/write', 'peer', DeviceLibraryLevel.readWrite),
      library('/unknown', 'peer', null),
    ];
    expect(currentLibraryIsReadOnly('/owned', libraries), false);
    expect(currentLibraryIsReadOnly('/read', libraries), true);
    expect(currentLibraryIsReadOnly('/write', libraries), false);
    expect(currentLibraryIsReadOnly('/unknown', libraries), false);
    expect(currentLibraryIsReadOnly('/missing', libraries), false);
    expect(currentLibraryIsReadOnly('@RR|/read', libraries), false);
  });
}
