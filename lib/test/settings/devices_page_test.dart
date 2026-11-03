import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/config/routes.dart';
import 'package:rune/l10n/app_localizations_en.dart';
import 'package:rune/screens/settings_devices/device_status.dart';
import 'package:rune/screens/settings_devices/settings_devices.dart';

void main() {
  test('device status describes each state and badge', () {
    final strings = AppLocalizationsEn();
    expect(deviceStatusPresentation(ClientStatus.approved, strings), (
      subtitle: 'My device',
      showBadge: false,
    ));
    expect(deviceStatusPresentation(ClientStatus.pending, strings), (
      subtitle: 'Waiting for approval',
      showBadge: true,
    ));
    expect(deviceStatusPresentation(ClientStatus.blocked, strings), (
      subtitle: 'Blocked',
      showBadge: true,
    ));
  });

  testWidgets('all device routes build the devices page', (tester) async {
    await tester.pumpWidget(
      Builder(
        builder: (context) {
          for (final path in [
            '/settings/devices',
            '/settings/neighbors',
            '/settings/server',
          ]) {
            expect(
              routes[path]!(context),
              isA<SettingsDevicesPage>(),
              reason: path,
            );
          }
          return const SizedBox();
        },
      ),
    );
  });
}
