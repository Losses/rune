import 'package:fluent_ui/fluent_ui.dart';

import '../../../../bindings/bindings.dart';

class ScrobbleLoginController {
  final TextEditingController usernameController = TextEditingController();
  final TextEditingController passwordController = TextEditingController();
  final TextEditingController apiKeyController = TextEditingController();
  final TextEditingController apiSecretController = TextEditingController();

  void dispose() {
    usernameController.dispose();
    passwordController.dispose();
    apiKeyController.dispose();
    apiSecretController.dispose();
  }

  LoginRequestItem toLoginRequestItem(String serviceName) {
    return LoginRequestItem(
      serviceId: serviceName,
      username: usernameController.text,
      password: passwordController.text,
      apiKey: apiKeyController.text,
      apiSecret: apiSecretController.text,
    );
  }
}
