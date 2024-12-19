import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/l10n.dart';
import '../../utils/dialogs/scrobble/show_scrobble_login_dialog.dart';

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

    return Button(
      child: Text(s.login),
      onPressed: () => showScrobbleLoginDialog(
        context,
        serviceName,
        title,
      ),
    );
  }
}
