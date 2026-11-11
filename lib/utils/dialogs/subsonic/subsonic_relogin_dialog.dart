import 'package:fluent_ui/fluent_ui.dart';

import '../../../bindings/bindings.dart';
import '../../l10n.dart';
import '../../router/navigation.dart';
import '../information/error.dart';
import '../scrobble/utils/scrobble_login_controller.dart';
import '../scrobble/widgets/scrobble_login_form.dart';
import '../../../providers/library_registry.dart';

Future<bool> showSubsonicReloginDialog(
  BuildContext context,
  LibraryRegistryProvider registry,
  String recordId,
) async {
  final controller = ScrobbleLoginController();
  try {
    return await $showModal<bool>(
          context,
          (context, close) => _SubsonicReloginDialog(
            controller: controller,
            registry: registry,
            recordId: recordId,
            close: close,
          ),
          barrierDismissible: true,
          dismissWithEsc: true,
        ) ??
        false;
  } finally {
    controller.dispose();
  }
}

class _SubsonicReloginDialog extends StatefulWidget {
  const _SubsonicReloginDialog({
    required this.controller,
    required this.registry,
    required this.recordId,
    required this.close,
  });

  final ScrobbleLoginController controller;
  final LibraryRegistryProvider registry;
  final String recordId;
  final void Function(bool?) close;

  @override
  State<_SubsonicReloginDialog> createState() => _SubsonicReloginDialogState();
}

class _SubsonicReloginDialogState extends State<_SubsonicReloginDialog> {
  bool _loading = false;

  Future<void> _login() async {
    setState(() => _loading = true);
    try {
      final result = await widget.registry.replaceSubsonicCredential(
        widget.recordId,
        widget.controller.usernameController.text,
        widget.controller.passwordController.text,
      );
      if (!mounted) return;
      if (result.result == ReplaceSubsonicCredentialResult.updated) {
        widget.close(true);
      } else {
        final s = S.of(context);
        await showErrorDialog(
          context: context,
          title: s.loginFailed,
          subtitle: s.loginFailedSubtitle,
          errorMessage: s.loginFailedSubtitle,
        );
      }
    } catch (_) {
      if (!mounted) return;
      final s = S.of(context);
      await showErrorDialog(
        context: context,
        title: s.loginFailed,
        subtitle: s.loginFailedSubtitle,
        errorMessage: s.loginFailedSubtitle,
      );
    } finally {
      if (mounted) setState(() => _loading = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    return ContentDialog(
      title: Text(s.retryLogin),
      content: ScrobbleLoginForm(
        serviceName: 'Subsonic',
        controller: widget.controller,
      ),
      actions: [
        FilledButton(onPressed: _loading ? null : _login, child: Text(s.login)),
        Button(
          onPressed: _loading ? null : () => widget.close(false),
          child: Text(s.cancel),
        ),
      ],
    );
  }
}

Future<void> openLibraryWithSubsonicRelogin(
  BuildContext context,
  LibraryRegistryProvider registry,
  String recordId,
) async {
  try {
    await registry.open(recordId);
  } on LibraryRegistryFailure catch (error) {
    if (error.kind != OpenLibraryFailureKind.remoteNotAuthorized ||
        !context.mounted) {
      rethrow;
    }
    final updated = await showSubsonicReloginDialog(
      context,
      registry,
      recordId,
    );
    if (!updated) rethrow;
    if (!context.mounted) return;
    await registry.open(recordId);
  }
}
