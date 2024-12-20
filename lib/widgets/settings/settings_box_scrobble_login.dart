import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:collection/collection.dart';

import '../../utils/dialogs/information/information.dart';
import '../../utils/l10n.dart';
import '../../utils/dialogs/scrobble/show_scrobble_login_dialog.dart';
import '../../providers/scrobble.dart';

import 'settings_box_base.dart';

class SettingsBoxScrobbleLogin extends SettingsBoxBase {
  const SettingsBoxScrobbleLogin({
    super.key,
    required super.title,
    required super.subtitle,
    required this.serviceName,
  });

  final String serviceName;

  @override
  Widget buildExpanderContent(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [],
    );
  }

  @override
  Widget buildDefaultContent(BuildContext context) {
    final s = S.of(context);
    final scrobbleProvider = Provider.of<ScrobbleProvider>(context);

    final serviceStatus = scrobbleProvider.serviceStatuses
        .firstWhereOrNull((status) => status.serviceId == serviceName);

    bool isLoggedIn = serviceStatus != null && serviceStatus.isAvailable;
    bool hasError = serviceStatus != null && serviceStatus.error.isNotEmpty;

    return Button(
      onPressed: isLoggedIn
          ? () => _showLogoutConfirmation(context)
          : () {
              if (hasError) {
                _showErrorOptionsMenu(context);
              } else {
                showScrobbleLoginDialog(context, serviceName, title);
              }
            },
      child: Text(
        isLoggedIn ? s.logout : (hasError ? s.fix : s.login),
      ),
    );
  }

  void _showLogoutConfirmation(BuildContext context) {
    final s = S.of(context);
    showInformationDialog(
      context: context,
      title: s.confirmLogoutTitle,
      subtitle: s.confirmLogoutSubtitle,
    ).then((confirmed) {
      if (confirmed == true) {
        if (!context.mounted) return;

        Provider.of<ScrobbleProvider>(context, listen: false)
            .logout(serviceName);
      }
    });
  }

  void _showErrorOptionsMenu(BuildContext context) {
    final s = S.of(context);
    final menuController = FlyoutController();

    menuController.showFlyout(
      builder: (context) {
        return MenuFlyout(
          items: [
            MenuFlyoutItem(
              text: Text(s.retryLogin),
              onPressed: () {
                Navigator.pop(context);
                showScrobbleLoginDialog(context, serviceName, title);
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
                showScrobbleLoginDialog(context, serviceName, title);
              },
            ),
          ],
        );
      },
    );
  }
}
