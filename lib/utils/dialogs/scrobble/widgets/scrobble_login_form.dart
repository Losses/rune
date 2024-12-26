import 'package:fluent_ui/fluent_ui.dart';

import '../../../l10n.dart';

import '../utils/scrobble_login_controller.dart';

class ScrobbleLoginForm extends StatelessWidget {
  final String serviceName;
  final ScrobbleLoginController controller;

  const ScrobbleLoginForm({
    super.key,
    required this.serviceName,
    required this.controller,
  });

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (serviceName != 'ListenBrainz') ...[
          InfoLabel(
            label: s.username,
            child: TextBox(controller: controller.usernameController),
          ),
          const SizedBox(height: 16),
        ],
        InfoLabel(
          label: serviceName == 'ListenBrainz' ? s.userToken : s.password,
          child: TextBox(
              controller: controller.passwordController, obscureText: true),
        ),
        if (serviceName == 'LastFm') ...[
          const SizedBox(height: 16),
          InfoLabel(
            label: s.apiKey,
            child: TextBox(controller: controller.apiKeyController),
          ),
          const SizedBox(height: 16),
          InfoLabel(
            label: s.apiSecret,
            child: TextBox(controller: controller.apiSecretController),
          ),
        ],
      ],
    );
  }
}
