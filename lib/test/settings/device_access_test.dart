import 'package:flutter_test/flutter_test.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations_en.dart';
import '../../screens/settings_devices/device_access.dart';

void main() {
  final s = AppLocalizationsEn();
  DeviceGrantsInfo grants({
    bool control = true,
    DeviceConnectionLevel? allConnection = DeviceConnectionLevel.view,
    DeviceSyncLevel? allSync = DeviceSyncLevel.none,
    List<DeviceLibraryGrant> libraries = const [],
    bool administer = false,
  }) => DeviceGrantsInfo(
    control: control,
    allConnection: allConnection,
    allSync: allSync,
    libraries: libraries,
    administer: administer,
  );

  test('summarizes my device, guest, custom and missing settings', () {
    expect(
      deviceAccessSummary(
        grants(
          allConnection: DeviceConnectionLevel.viewEdit,
          allSync: DeviceSyncLevel.pullPush,
        ),
        s,
      ),
      'My device',
    );
    expect(deviceAccessSummary(grants(), s), 'Guest · Can control playback');
    expect(
      deviceAccessSummary(
        grants(
          libraries: [
            const DeviceLibraryGrant(
              libraryId: 'a',
              connection: DeviceConnectionLevel.view,
              sync: DeviceSyncLevel.none,
            ),
          ],
        ),
        s,
      ),
      'Guest · Can control playback',
    );
    expect(
      deviceAccessSummary(
        grants(
          libraries: [
            const DeviceLibraryGrant(
              libraryId: 'a',
              connection: DeviceConnectionLevel.none,
              sync: DeviceSyncLevel.none,
            ),
          ],
        ),
        s,
      ),
      'Custom',
    );
    expect(deviceAccessSummary(grants(control: false), s), 'Custom');
    expect(
      deviceAccessSummary(
        grants(
          allConnection: DeviceConnectionLevel.viewEdit,
          allSync: DeviceSyncLevel.pullPush,
          libraries: [
            const DeviceLibraryGrant(
              libraryId: 'a',
              connection: DeviceConnectionLevel.view,
              sync: DeviceSyncLevel.pull,
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
