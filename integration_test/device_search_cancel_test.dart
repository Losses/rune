import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:rinf/rinf.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/screens/settings_neighbors/utils/show_search_remote_device_dialog.dart';
import 'package:rune/screens/settings_neighbors/widgets/search_remote_device_dialog.dart';
import 'package:rune/utils/add_library_flows.dart';
import 'package:rune/utils/api/request_failure.dart';

Future<void> _pumpUntil(
  WidgetTester tester,
  bool Function() condition,
  String description,
) async {
  for (var frame = 0; frame < 60 && !condition(); frame++) {
    await tester.pump(const Duration(milliseconds: 16));
  }
  expect(condition(), isTrue, reason: description);
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  testWidgets('closed device searches complete pairing and can restart', (
    tester,
  ) async {
    await tester.runAsync(() async {
      final directory = await Directory.systemTemp.createTemp('rune-pairing');
      addTearDown(() => directory.delete(recursive: true));
      await initializeRust(assignRustSignal);
      addTearDown(finalizeRust);
      await awaitResponse(
        'InitializeNodeRequest',
        InitializeNodeResponse.rustSignalStream,
        responseCallId: (response) => response.message.callId?.toInt(),
        send: (callId) => InitializeNodeRequest(
          configPath: directory.path,
          callId: callId,
        ).sendSignalToRust(),
      );
    });

    var completed = 0;
    var trustRequests = 0;
    var registrations = 0;
    final results = <(DiscoveredDeviceMessage, bool?)?>[];
    await tester.pumpWidget(
      FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Builder(
          builder: (context) => Button(
            child: const Text('Find devices'),
            onPressed: () async {
              await pairRuneDevice(
                context,
                showSearch: (context) async {
                  final result = await showSearchRemoteDeviceDialog(context);
                  results.add(result);
                  return result;
                },
                trustDevice: (_, _) async => trustRequests++,
                registerDevice: (_, _) async => registrations++,
              );
              completed++;
            },
          ),
        ),
      ),
    );

    final dialog = find.byType(SearchRemoteDeviceDialog);
    for (final expected in [1, 2, 3]) {
      await tester.tap(find.text('Find devices'));
      await _pumpUntil(
        tester,
        () => dialog.evaluate().isNotEmpty,
        'Search opened',
      );
      if (expected == 3) {
        tester.state<NavigatorState>(find.byType(Navigator)).pop();
      } else {
        final cancel = find.widgetWithText(Button, 'Cancel');
        final closeAgain = tester.widget<Button>(cancel).onPressed!;
        await tester.tap(cancel);
        if (expected == 2) closeAgain();
      }
      await _pumpUntil(
        tester,
        () => completed == expected,
        'Closing search should complete the pairing future',
      );
      await tester.pumpAndSettle();
      expect(dialog, findsNothing);
      expect(results.length, expected);
      expect(results.last, isNull);
      expect(trustRequests, 0);
      expect(registrations, 0);
    }
  });
}
