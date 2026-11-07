import 'package:flutter_test/flutter_test.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations_en.dart';
import '../../screens/settings_devices/device_access.dart';

void main() {
  final s = AppLocalizationsEn();
  DeviceGrantsInfo grants({
    bool control = true,
    DeviceLibraryLevel? all,
    List<DeviceLibraryGrant> libraries = const [],
    bool administer = false,
  }) => DeviceGrantsInfo(
    control: control,
    allLibraries: all,
    libraries: libraries,
    administer: administer,
  );

  test('summarizes my device, guest, custom and missing settings', () {
    expect(
      deviceAccessSummary(grants(all: DeviceLibraryLevel.readWrite), s),
      'My device',
    );
    expect(deviceAccessSummary(grants(), s), 'Guest · Can control playback');
    expect(
      deviceAccessSummary(
        grants(
          libraries: [
            const DeviceLibraryGrant(
              libraryId: 'a',
              level: DeviceLibraryLevel.notShared,
            ),
          ],
        ),
        s,
      ),
      'Guest · Can control playback',
    );
    expect(deviceAccessSummary(grants(control: false), s), 'Custom');
    expect(
      deviceAccessSummary(
        grants(
          all: DeviceLibraryLevel.readWrite,
          libraries: [
            const DeviceLibraryGrant(
              libraryId: 'a',
              level: DeviceLibraryLevel.read,
            ),
          ],
        ),
        s,
      ),
      'Custom',
    );
    expect(deviceAccessSummary(grants(administer: true), s), 'Custom');
    expect(deviceAccessSummary(null, s), 'Custom');
  });

  test('library levels map to the corresponding dropdown values', () {
    expect(
      choiceForLevel(DeviceLibraryLevel.notShared),
      DeviceAccessChoice.notShared,
    );
    expect(choiceForLevel(DeviceLibraryLevel.read), DeviceAccessChoice.read);
    expect(
      choiceForLevel(DeviceLibraryLevel.readWrite),
      DeviceAccessChoice.readWrite,
    );
    for (final choice in DeviceAccessChoice.values) {
      expect(choiceForLevel(levelForChoice(choice)), choice);
    }
  });

  test('only approved controlling devices show the live status', () {
    expect(
      deviceRowSubtitle(
        status: ClientStatus.approved,
        grants: grants(),
        isControlling: true,
        strings: s,
      ),
      'Guest · Can control playback · Controlling now',
    );
    expect(
      deviceRowSubtitle(
        status: ClientStatus.approved,
        grants: grants(),
        isControlling: false,
        strings: s,
      ),
      'Guest · Can control playback',
    );
    expect(
      deviceRowSubtitle(
        status: ClientStatus.pending,
        grants: grants(),
        isControlling: true,
        strings: s,
      ),
      s.waitingForApproval,
    );
    expect(
      deviceRowSubtitle(
        status: ClientStatus.blocked,
        grants: grants(),
        isControlling: true,
        strings: s,
      ),
      s.blockedStatus,
    );
  });
}
