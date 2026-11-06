import 'package:fluent_ui/fluent_ui.dart';

class AddRemoteDeviceFormController {
  final TextEditingController hostnameController = TextEditingController();

  void dispose() {
    hostnameController.dispose();
  }
}
