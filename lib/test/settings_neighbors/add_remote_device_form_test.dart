import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/screens/settings_neighbors/widgets/add_remote_device_dialog.dart';

void main() {
  testWidgets('add device form uses host and port boxes without a secure switch',
      (tester) async {
    await tester.pumpWidget(
      FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: AddRemoteDeviceDialog(
          navigateIfFailed: false,
          $close: (_) {},
        ),
      ),
    );

    expect(find.byType(ToggleSwitch), findsNothing);
    final controller = tester
        .state<AddRemoteDeviceDialogState>(find.byType(AddRemoteDeviceDialog))
        .controller;
    expect(controller.portController.text, '7863');
    expect(controller.toDeviceAddress(), 'localhost:7863');
    controller.hostnameController.text = 'peer.example';
    controller.portController.text = '9000';
    expect(controller.toDeviceAddress(), 'peer.example:9000');
    controller.hostnameController.text = '::1';
    expect(controller.toDeviceAddress(), '[::1]:9000');
  });
}
