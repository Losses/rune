import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:collection/collection.dart';

import '../../utils/l10n.dart';
import '../../utils/router/router_aware_flyout_controller.dart';
import '../../utils/dialogs/scrobble/show_scrobble_login_dialog.dart';
import '../../utils/dialogs/information/confirm.dart';
import '../../utils/dialogs/information/error.dart';
import '../../providers/scrobble.dart';

import 'settings_box_base.dart';

class SettingsBoxScrobbleLogin extends StatefulWidget {
  const SettingsBoxScrobbleLogin({
    super.key,
    required this.title,
    required this.subtitle,
    required this.serviceId,
  });

  final String title;
  final String subtitle;

  final String serviceId;

  @override
  State<SettingsBoxScrobbleLogin> createState() =>
      _SettingsBoxScrobbleLoginState();
}

class _SettingsBoxScrobbleLoginState extends State<SettingsBoxScrobbleLogin> {
  final _menuController = RouterAwareFlyoutController();

  Widget buildExpanderContent(BuildContext context) {
    final s = S.of(context);
    final scrobbleProvider = Provider.of<ScrobbleProvider>(context);

    final serviceStatus = scrobbleProvider.serviceStatuses
        .firstWhereOrNull((status) => status.serviceId == widget.serviceId);

    bool isLoggedIn = serviceStatus != null && serviceStatus.isAvailable;
    bool hasError = serviceStatus != null && serviceStatus.error.isNotEmpty;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: isLoggedIn
          ? (hasError)
              ? [
                  Button(
                    child: Text(s.retryLogin),
                    onPressed: () => _retryLogin(context),
                  ),
                  Button(
                    child: Text(s.logout),
                    onPressed: () => _showLogoutConfirmation(context),
                  ),
                  Button(
                    child: Text(s.edit),
                    onPressed: () => showScrobbleLoginDialog(
                        context, widget.serviceId, widget.title),
                  ),
                ]
              : [
                  Button(
                    child: Text(s.logout),
                    onPressed: () => _showLogoutConfirmation(context),
                  )
                ]
          : [
              Button(
                child: Text(s.login),
                onPressed: () => showScrobbleLoginDialog(
                    context, widget.serviceId, widget.title),
              )
            ],
    );
  }

  Widget buildDefaultContent(BuildContext context) {
    final s = S.of(context);
    final scrobbleProvider = Provider.of<ScrobbleProvider>(context);

    final serviceStatus = scrobbleProvider.serviceStatuses
        .firstWhereOrNull((status) => status.serviceId == widget.serviceId);

    bool isLoggedIn = serviceStatus != null && serviceStatus.isAvailable;
    bool hasError = serviceStatus != null &&
        serviceStatus.error.isNotEmpty &&
        serviceStatus.hasCredentials;

    return FlyoutTarget(
      controller: _menuController.controller,
      child: Button(
        onPressed: isLoggedIn
            ? () => _showLogoutConfirmation(context)
            : () {
                if (hasError) {
                  _showErrorOptionsMenu(context);
                } else {
                  showScrobbleLoginDialog(
                      context, widget.serviceId, widget.title);
                }
              },
        child: Text(
          isLoggedIn ? s.logout : (hasError ? s.fix : s.login),
        ),
      ),
    );
  }

  void _retryLogin(BuildContext context) async {
    final s = S.of(context);
    final scrobbleProvider =
        Provider.of<ScrobbleProvider>(context, listen: false);

    try {
      await scrobbleProvider.retryLogin(widget.serviceId);
    } catch (e) {
      if (!context.mounted) return;
      showErrorDialog(
        context: context,
        title: s.loginFailed,
        subtitle: s.loginFailedSubtitle,
        errorMessage: e.toString(),
      );
    }
  }

  void _showLogoutConfirmation(BuildContext context) {
    final s = S.of(context);
    showConfirmDialog(
      context: context,
      title: s.confirmLogoutTitle,
      subtitle: s.confirmLogoutSubtitle,
      yesLabel: s.confirm,
      noLabel: s.cancel,
    ).then((confirmed) {
      if (confirmed == true) {
        if (!context.mounted) return;

        Provider.of<ScrobbleProvider>(context, listen: false)
            .logout(widget.serviceId);
      }
    });
  }

  @override
  dispose() {
    _menuController.dispose();
    super.dispose();
  }

  void _showErrorOptionsMenu(BuildContext context) {
    final s = S.of(context);

    _menuController.showFlyout(
      builder: (context) {
        return MenuFlyout(
          items: [
            MenuFlyoutItem(
              text: Text(s.retryLogin),
              onPressed: () {
                Navigator.pop(context);
                _retryLogin(context);
              },
            ),
            MenuFlyoutItem(
              text: Text(s.logout),
              onPressed: () {
                Navigator.pop(context);
                _showLogoutConfirmation(context);
              },
            ),
            MenuFlyoutItem(
              text: Text(s.edit),
              onPressed: () {
                Navigator.pop(context);
                showScrobbleLoginDialog(
                    context, widget.serviceId, widget.title);
              },
            ),
          ],
        );
      },
    );
  }

  @override
  Widget build(BuildContext context) {
    return SettingsBoxBase(
      title: widget.title,
      subtitle: widget.subtitle,
      buildExpanderContent: buildExpanderContent,
      buildDefaultContent: buildDefaultContent,
    );
  }
}
