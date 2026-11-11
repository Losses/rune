import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/config/routes.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/l10n/app_localizations_en.dart';
import 'package:rune/providers/controllers.dart';
import 'package:rune/screens/settings_devices/device_status.dart';
import 'package:rune/screens/settings_devices/settings_devices.dart';

void main() {
  test('device status describes each state and badge', () {
    final strings = AppLocalizationsEn();
    expect(deviceStatusPresentation(ClientStatus.approved, strings), (
      subtitle: 'My Device',
      showBadge: true,
    ));
    expect(deviceStatusPresentation(ClientStatus.pending, strings), (
      subtitle: 'Waiting for Approval',
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

  testWidgets(
    'Disconnect appears only for a controlling device and detaches each connection',
    (tester) async {
      final changes = StreamController<ControllersUpdated>(sync: true);
      final detachedIds = <String>[];
      final controllers = ControllersProvider(
        load: () async => [
          AttachedControllerInfo(
            id: 'connection-1',
            fingerprint: 'device-a',
            alias: 'Controller',
            connectedAt: Uint64.fromBigInt(BigInt.one),
          ),
          AttachedControllerInfo(
            id: 'connection-2',
            fingerprint: 'device-a',
            alias: 'Controller',
            connectedAt: Uint64.fromBigInt(BigInt.from(2)),
          ),
        ],
        changes: changes.stream,
        detach: (id) async => detachedIds.add(id),
      );
      addTearDown(controllers.dispose);
      addTearDown(changes.close);

      await tester.pumpWidget(
        ChangeNotifierProvider<ControllersProvider>.value(
          value: controllers,
          child: const FluentApp(
            localizationsDelegates: AppLocalizations.localizationsDelegates,
            supportedLocales: AppLocalizations.supportedLocales,
            home: Center(
              child: Row(
                children: [
                  DisconnectControllerAction(fingerprint: 'device-a'),
                  DisconnectControllerAction(fingerprint: 'device-b'),
                ],
              ),
            ),
          ),
        ),
      );
      await tester.pump();
      expect(find.text('Disconnect'), findsOneWidget);
      await tester.tap(find.text('Disconnect'));
      await tester.pump(const Duration(milliseconds: 100));
      expect(detachedIds, ['connection-1', 'connection-2']);

      changes.add(const ControllersUpdated(controllers: []));
      await tester.pump();
      expect(find.text('Disconnect'), findsNothing);
    },
  );
}
