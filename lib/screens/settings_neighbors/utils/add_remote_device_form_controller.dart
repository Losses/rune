import 'package:fluent_ui/fluent_ui.dart';
import '../../../utils/dialogs/mix/utils/toggle_switch_controller.dart';

class AddRemoteDeviceFormController {
  final TextEditingController hostnameController = TextEditingController();
  final TextEditingController portController = TextEditingController();
  final ToggleSwitchController securedController = ToggleSwitchController();

  static const String defaultHostname = 'localhost';
  static const String defaultPort = '7863';
  static const bool defaultSecured = false;

  AddRemoteDeviceFormController() {
    hostnameController.text = defaultHostname;
    portController.text = defaultPort;
    securedController.isChecked = defaultSecured;
  }

  factory AddRemoteDeviceFormController.fromWebSocketUrl(String url) {
    final controller = AddRemoteDeviceFormController();

    try {
      final uri = Uri.parse(url);

      if (uri.scheme == 'ws' || uri.scheme == 'wss') {
        controller.securedController.isChecked = (uri.scheme == 'wss');

        controller.hostnameController.text =
            uri.host.isNotEmpty ? uri.host : defaultHostname;

        controller.portController.text =
            uri.port != 0 ? uri.port.toString() : defaultPort;
      } else {
        throw 'Invalid URL';
      }
    } catch (e) {
      controller.hostnameController.text = defaultHostname;
      controller.portController.text = defaultPort;
      controller.securedController.isChecked = defaultSecured;
    }

    return controller;
  }

  String toWebSocketUrl() {
    final protocol = securedController.isChecked ? 'wss' : 'ws';
    final address = hostnameController.text.isNotEmpty
        ? hostnameController.text
        : defaultHostname;
    final port =
        portController.text.isNotEmpty ? portController.text : defaultPort;

    return '$protocol://$address:$port';
  }

  void dispose() {
    hostnameController.dispose();
    portController.dispose();
    securedController.dispose();
  }
}
