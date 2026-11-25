import 'package:fluent_ui/fluent_ui.dart';

class AddRemoteDeviceFormController {
  final TextEditingController hostnameController = TextEditingController();
  final TextEditingController portController = TextEditingController();
  static const String defaultHostname = 'localhost';
  static const String defaultPort = '7863';

  AddRemoteDeviceFormController() {
    hostnameController.text = defaultHostname;
    portController.text = defaultPort;
  }

  String toDeviceAddress() {
    final address = hostnameController.text.isNotEmpty
        ? hostnameController.text
        : defaultHostname;
    final formattedAddress = address.contains(':') && !address.startsWith('[')
        ? '[$address]'
        : address;
    final port = portController.text;

    return port.isEmpty ? formattedAddress : '$formattedAddress:$port';
  }

  void dispose() {
    hostnameController.dispose();
    portController.dispose();
  }
}
