import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:collection/collection.dart';

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
          ? null
          : () => showScrobbleLoginDialog(
                context,
                serviceName,
                title,
              ),
      child: Text(
        isLoggedIn ? s.edit : (hasError ? s.fix : s.login),
      ),
    );
  }
}
