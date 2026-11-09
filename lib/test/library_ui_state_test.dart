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
        libraryId: '1',
        alias: 'One',
        added: false,
        connection: DeviceConnectionLevel.view,
        sync: DeviceSyncLevel.pull,
      ),
      PeerLibrary(
        libraryId: '2',
        alias: 'Two',
        added: true,
        connection: DeviceConnectionLevel.viewEdit,
        sync: DeviceSyncLevel.pullPush,
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
        connectionAccess: DeviceConnectionLevel.view,
        syncAccess: DeviceSyncLevel.pull,
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
      path,
      libraries,
      aliases,
      (device) => 'From $device',
      s.readOnly,
    );
    expect(row('/one', {'peer': 'Laptop'}).subtitle, 'From Laptop · Read only');
    expect(row('/one').subtitle, 'From Music · Read only');
    expect(row('/one').library?.backgroundSync, true);
    expect(row('/two').subtitle, '/two');
    expect(row('/two').library?.backgroundSync, false);
    expect(row('/missing').library, isNull);
  });
  test('iOS folders are named by where the Files app shows them', () {
    String location(String path) => filesAppLocation(
      path,
      onThisDevice: 'On My iPhone',
      iCloudDrive: 'iCloud Drive',
    );
    const onDevice =
        '/private/var/mobile/Containers/Shared/AppGroup/'
        '2A36B718-E77C-43AD-9B4A-C15B1D7F74FC/File Provider Storage';
    expect(location('$onDevice/music'), 'On My iPhone › music');
    expect(
      location('${onDevice.substring('/private'.length)}/Music/Jazz'),
      'On My iPhone › Music › Jazz',
    );
    expect(location(onDevice), 'On My iPhone');
    expect(
      location(
        '/private/var/mobile/Library/Mobile Documents/com~apple~CloudDocs/Music',
      ),
      'iCloud Drive › Music',
    );
    expect(location('/Users/me/Music'), '/Users/me/Music');
  });
  test('a local row shows the described path, a shared one its device', () {
    final libraries = [
      LocalLibrary(
        libraryId: 'a',
        alias: 'Music',
        path: '/one',
        ownerFingerprint: 'peer',
        backgroundSync: true,
      ),
      LocalLibrary(
        libraryId: 'b',
        alias: 'Local',
        path: '/two',
        backgroundSync: false,
      ),
    ];
    LibraryRowState row(String path) => libraryRowState(
      path,
      libraries,
      const {},
      (device) => 'From $device',
      s.readOnly,
      describePath: (path) => 'described $path',
    );
    expect(row('/two').subtitle, 'described /two');
    expect(row('/missing').subtitle, 'described /missing');
    expect(row('/one').subtitle, startsWith('From Music'));
  });
}
