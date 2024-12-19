import 'package:fluent_ui/fluent_ui.dart';

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
    return Column(
      children: [
        if (serviceName != 'ListenBrainz') ...[
          InfoLabel(
            label: 'Username',
            child: TextBox(controller: controller.usernameController),
          ),
          const SizedBox(height: 16),
        ],
        InfoLabel(
          label: 'Password',
          child: TextBox(
              controller: controller.passwordController, obscureText: true),
        ),
        if (serviceName == 'LastFm') ...[
          const SizedBox(height: 16),
          InfoLabel(
            label: 'API Key',
            child: TextBox(controller: controller.apiKeyController),
          ),
          const SizedBox(height: 16),
          InfoLabel(
            label: 'API Secret',
            child: TextBox(controller: controller.apiSecretController),
          ),
        ],
      ],
    );
  }
}
