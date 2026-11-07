import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import 'device_status.dart';

enum DeviceAccessChoice { notShared, read, readWrite }

DeviceAccessChoice choiceForLevel(DeviceSyncLevel level) => switch (level) {
  DeviceSyncLevel.none => DeviceAccessChoice.notShared,
  DeviceSyncLevel.pull => DeviceAccessChoice.read,
  DeviceSyncLevel.pullPush => DeviceAccessChoice.readWrite,
};

DeviceSyncLevel levelForChoice(DeviceAccessChoice choice) => switch (choice) {
  DeviceAccessChoice.notShared => DeviceSyncLevel.none,
  DeviceAccessChoice.read => DeviceSyncLevel.pull,
  DeviceAccessChoice.readWrite => DeviceSyncLevel.pullPush,
};

String deviceAccessSummary(DeviceGrantsInfo? grants, AppLocalizations s) {
  if (grants == null) return s.customDeviceAccess;
  if (grants.control && !grants.administer) {
    if (grants.allConnection == DeviceConnectionLevel.viewEdit &&
        grants.allSync == DeviceSyncLevel.pullPush &&
        grants.libraries.every(
          (entry) =>
              entry.connection == DeviceConnectionLevel.viewEdit &&
              entry.sync == DeviceSyncLevel.pullPush,
        )) {
      return s.myDevice;
    }
    if (grants.allConnection == DeviceConnectionLevel.view &&
        grants.allSync == DeviceSyncLevel.none &&
        grants.libraries.every(
          (entry) =>
              entry.connection == DeviceConnectionLevel.none &&
              entry.sync == DeviceSyncLevel.none,
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
