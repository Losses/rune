import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../providers/library_registry.dart';
import '../device_sign_in.dart';
import 'device_sign_in/device_sign_in_dialog.dart';
import 'subsonic/subsonic_relogin_dialog.dart';

/// Opens the library of [recordId] with the sign-in it needs. A library that signs in with a
/// credential it keeps opens as [openLibraryWithSubsonicRelogin] opens it. A library whose
/// sign-in lives on this device signs in first when it says it is not signed in.
///
/// True once it is open. False when its sign-in on this device needs the user, who was told
/// why and closed that, so nothing more is to say. A library whose sign-in failed for a reason
/// that may pass by itself opens all the same, from what it kept. A failure to open throws.
Future<bool> openLibraryWithSignIn(
  BuildContext context,
  LibraryRegistryProvider registry,
  String recordId,
) async {
  final signedIn = registry.records
      .where((record) => record.recordId == recordId)
      .firstOrNull
      ?.signedIn;
  if (signedIn == null) {
    await openLibraryWithSubsonicRelogin(context, registry, recordId);
    return true;
  }
  if (!signedIn) {
    final device = context.read<DeviceSignIns>();
    if (device.canSignIn(recordId)) {
      final canOpen = await signInOnDeviceAgain(context, device, recordId);
      if (!canOpen || !context.mounted) return false;
    }
  }
  await registry.open(recordId);
  return true;
}

/// Signs a library in on this device when something the user did not ask for shows that it
/// needs a sign-in: a request that failed, or a platform host that lost its access.
///
/// One flow runs per library at a time, and what comes in meanwhile is left to it. The library
/// signs in by itself and shows nothing, within the cooldown of the policy. When only the user
/// can make the sign-in work, the open library is closed and the library chooser tells why. A
/// sign-in that failed for a reason that may pass is tried again when the network is back.
class AutomaticDeviceSignIn {
  AutomaticDeviceSignIn({
    required this.signIns,
    required this.dialogContext,
    required this.leaveLibrary,
    required this.showLibrary,
  });

  final DeviceSignIns signIns;

  /// Where the flow shows its dialogs, or null when there is nowhere to show them now.
  final BuildContext? Function() dialogContext;

  /// Closes the open library and shows the library chooser.
  final Future<void> Function() leaveLibrary;

  /// Shows the pages of the library that was just opened from the library chooser.
  final void Function() showLibrary;

  final _running = <String, Future<void>>{};

  /// Starts the flow for the library of [recordId], which a request found not signed in. True
  /// when the library signs in on this device, so that the failure is left to the flow.
  bool needsSignIn(String recordId) {
    if (signIns.methodOf(recordId) == null) return false;
    _start(recordId);
    return true;
  }

  /// The host of [family] lost its access, so the open library is signed in again when it
  /// signs in through that host. Any other library open leaves it to the next open.
  void accessLost(String family) {
    final recordId = signIns.registry.openRecordId;
    if (recordId != null && signIns.methodOf(recordId)?.host == family) {
      _start(recordId);
    }
  }

  /// A host reported the network back, so the libraries whose sign-in failed for a reason that
  /// may have passed with it are signed in again, cooldown or not.
  void networkBack() {
    for (final recordId in signIns.policy.awaitingRetry) {
      _start(recordId, afterCooldown: false);
    }
  }

  void _start(String recordId, {bool afterCooldown = true}) {
    if (!signIns.canSignIn(recordId) ||
        _running.containsKey(recordId) ||
        (afterCooldown && !signIns.policy.worthTrying(recordId))) {
      return;
    }
    _running[recordId] = _signIn(recordId).whenComplete(() {
      // A callback that returned the removed future would wait for itself.
      _running.remove(recordId);
    });
  }

  Future<void> _signIn(String recordId) async {
    final failure = await signIns.signIn(recordId);
    if (failure == null ||
        !failure.userMustAct ||
        signIns.registry.openRecordId != recordId) {
      return;
    }
    await leaveLibrary();
    final context = dialogContext();
    if (context == null || !context.mounted) return;
    await reopenAfterDeviceSignIn(
      context,
      signIns,
      recordId,
      failure,
      showLibrary: showLibrary,
    );
  }
}
