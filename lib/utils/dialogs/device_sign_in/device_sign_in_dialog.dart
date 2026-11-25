import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../../../providers/library_registry.dart';
import '../../device_sign_in.dart';
import '../../l10n.dart';
import '../../router/navigation.dart';
import '../failed_to_initialize_library.dart';

/// Signs the library of [recordId] in again by itself, and answers whether the
/// library can open: once it is signed in, and also when the sign-in failed for
/// a reason that may pass by itself, as offline, since the library then shows
/// what it kept and signs in by itself once the network is back or a later
/// request needs it. Only when the user must act does it tell why with
/// [showDeviceSignInFailure], and the library can open once a Retry there signs
/// it in. A flow already under way for the record, dialog included, is shared.
///
/// Without a host there is nothing to sign in with: it returns false and shows
/// nothing, leaving the caller to say why the library cannot be used.
Future<bool> signInOnDeviceAgain(
  BuildContext context,
  DeviceSignIns signIns,
  String recordId,
) {
  if (!signIns.canSignIn(recordId)) return Future.value(false);
  return signIns.shared(recordId, () async {
    final failure = await signIns.signIn(recordId);
    if (failure == null || !failure.userMustAct) return true;
    if (!context.mounted) return false;
    return showDeviceSignInFailure(context, signIns, recordId, failure);
  });
}

/// Tells on the library chooser why the library of [recordId] did not sign in,
/// as [showDeviceSignInFailure] does. A Retry that signs in opens the library,
/// and [showLibrary] shows its pages. A flow already under way for the record
/// is shared.
Future<void> reopenAfterDeviceSignIn(
  BuildContext context,
  DeviceSignIns signIns,
  String recordId,
  DeviceSignInFailure failure, {
  required void Function() showLibrary,
}) => signIns.shared(recordId, () async {
  if (!await showDeviceSignInFailure(context, signIns, recordId, failure)) {
    return false;
  }
  try {
    await signIns.registry.open(recordId);
  } catch (error) {
    if (context.mounted) {
      await showFailedToInitializeLibrary(
        context,
        libraryFailureMessage(S.of(context), error),
      );
    }
    return false;
  }
  showLibrary();
  return true;
});

/// Tells the user why the library of [recordId] did not sign in, with Retry and
/// Close. Retry signs in again, which lets the system ask for access where it
/// never did, and the dialog tells the new reason when that fails too. True
/// once the library is signed in, false when the user closed the dialog.
Future<bool> showDeviceSignInFailure(
  BuildContext context,
  DeviceSignIns signIns,
  String recordId,
  DeviceSignInFailure failure,
) async {
  final method = signIns.methodOf(recordId);
  if (method == null) return false;
  // The latest sign-in the dialog ran. The dialog may be gone, and the modal
  // answered with nothing, before it ends, so it is waited for rather than
  // counted as closed.
  Future<bool>? attempt;
  final shown = await $showModal<bool>(
    context,
    (context, close) => _DeviceSignInFailureDialog(
      method: method,
      failure: failure,
      signIn: () => signIns.signIn(recordId),
      close: close,
      onAttempt: (running) => attempt = running,
    ),
    // Esc and a click beside the dialog go through the route, which asks the
    // dialog's own scope: neither closes it while it signs in.
    barrierDismissible: true,
  );
  var signedIn = shown ?? false;
  if (!signedIn && attempt != null) signedIn = await attempt!;
  return signedIn;
}

class _DeviceSignInFailureDialog extends StatefulWidget {
  const _DeviceSignInFailureDialog({
    required this.method,
    required this.failure,
    required this.signIn,
    required this.close,
    required this.onAttempt,
  });

  final DeviceSignInMethod method;
  final DeviceSignInFailure failure;
  final Future<DeviceSignInFailure?> Function() signIn;
  final void Function(bool?) close;

  /// Gets each sign-in the dialog starts, which ends true once signed in.
  final void Function(Future<bool> attempt) onAttempt;

  @override
  State<_DeviceSignInFailureDialog> createState() =>
      _DeviceSignInFailureDialogState();
}

class _DeviceSignInFailureDialogState
    extends State<_DeviceSignInFailureDialog> {
  late DeviceSignInFailure _failure = widget.failure;
  bool _loading = false;

  Future<void> _retry() async {
    setState(() => _loading = true);
    final attempt = Completer<bool>();
    widget.onAttempt(attempt.future);
    final failure = await widget.signIn();
    attempt.complete(failure == null);
    if (!mounted) return;
    if (failure == null) {
      widget.close(true);
      return;
    }
    setState(() {
      _failure = failure;
      _loading = false;
    });
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    return PopScope(
      canPop: !_loading,
      child: ContentDialog(
        title: Column(
          children: [const SizedBox(height: 8), Text(s.loginFailed)],
        ),
        content: SelectableText(
          widget.method.message(s, _failure),
          style: const TextStyle(height: 1.4),
        ),
        actions: [
          FilledButton(
            onPressed: _loading ? null : _retry,
            child: Text(s.playOnRetry),
          ),
          Button(
            onPressed: _loading ? null : () => widget.close(false),
            child: Text(s.close),
          ),
        ],
      ),
    );
  }
}
