import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../l10n/app_localizations.dart';
import '../../providers/broadcast.dart';
import '../../screens/settings_neighbors/widgets/edit_device_information_setting_dialog.dart';

class _Broadcast extends ChangeNotifier implements BroadcastProvider {
  _Broadcast(this.deviceAlias);

  @override
  String? deviceAlias;

  @override
  String? get fingerprint => null;

  @override
  Future<void> updateDeviceAlias(String newAlias) async {
    deviceAlias = newAlias;
    notifyListeners();
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  for (final initialName in ['Office', '']) {
    testWidgets('Edit follows a device name starting at "$initialName"', (
      tester,
    ) async {
      final broadcast = _Broadcast(initialName);
      addTearDown(broadcast.dispose);
      await tester.pumpWidget(
        FluentApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: ChangeNotifierProvider<BroadcastProvider>.value(
            value: broadcast,
            child: EditDeviceInformationSettingDialog($close: (_) {}),
          ),
        ),
      );

      final input = find.byType(TextBox);
      final edit = find.widgetWithText(FilledButton, 'Edit');
      bool canSave() => tester.widget<FilledButton>(edit).onPressed != null;

      expect(canSave(), initialName.isNotEmpty);
      await tester.enterText(input, initialName.isEmpty ? 'Studio' : '');
      await tester.pump();
      expect(
        canSave(),
        initialName.isEmpty,
        reason: 'Edit should update as the name changes',
      );
      await tester.enterText(input, '  ');
      await tester.pump();
      expect(canSave(), isFalse);
      expect(broadcast.deviceAlias, initialName);

      await tester.enterText(input, 'Studio');
      await tester.pump();
      expect(canSave(), isTrue);
      await tester.tap(edit);
      await tester.pumpAndSettle();
      expect(broadcast.deviceAlias, 'Studio');
    });
  }
}
