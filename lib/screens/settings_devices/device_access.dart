import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import 'device_status.dart';

enum DeviceAccessChoice { notShared, read, readWrite }

DeviceAccessChoice choiceForLevel(DeviceLibraryLevel level) => switch (level) {
  DeviceLibraryLevel.notShared => DeviceAccessChoice.notShared,
  DeviceLibraryLevel.read => DeviceAccessChoice.read,
  DeviceLibraryLevel.readWrite => DeviceAccessChoice.readWrite,
};

DeviceLibraryLevel levelForChoice(DeviceAccessChoice choice) =>
    switch (choice) {
      DeviceAccessChoice.notShared => DeviceLibraryLevel.notShared,
      DeviceAccessChoice.read => DeviceLibraryLevel.read,
      DeviceAccessChoice.readWrite => DeviceLibraryLevel.readWrite,
    };

String deviceAccessSummary(DeviceGrantsInfo? grants, AppLocalizations s) {
  if (grants == null) return s.customDeviceAccess;
  if (grants.control && !grants.administer) {
    if (grants.allLibraries == DeviceLibraryLevel.readWrite &&
        grants.libraries.every(
          (entry) => entry.level == DeviceLibraryLevel.readWrite,
        )) {
      return s.myDevice;
    }
    if ((grants.allLibraries == null ||
            grants.allLibraries == DeviceLibraryLevel.notShared) &&
        grants.libraries.every(
          (entry) => entry.level == DeviceLibraryLevel.notShared,
        )) {
      return s.guestControlSummary;
    }
  }
  return s.customDeviceAccess;
}

String deviceRowSubtitle({
  required ClientStatus status,
  required DeviceGrantsInfo? grants,
  required bool isControlling,
  required AppLocalizations strings,
}) {
  if (status != ClientStatus.approved) {
    return deviceStatusPresentation(status, strings).subtitle;
  }
  final access = deviceAccessSummary(grants, strings);
  return isControlling ? '$access · ${strings.controllingNow}' : access;
}
