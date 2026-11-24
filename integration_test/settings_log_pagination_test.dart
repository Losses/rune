import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

import 'package:rune/bindings/bindings.dart';
import 'package:rune/main.dart' as app;
import 'package:rune/screens/home/widgets/welcome.dart';
import 'package:rune/screens/settings_home/settings_home.dart';
import 'package:rune/screens/settings_log/settings_log.dart';
import 'package:rune/screens/settings_log/widgets/log_item.dart';
import 'package:rune/utils/api/list_logs.dart';
import 'package:rune/utils/api/request_failure.dart';
import 'package:rune/utils/l10n.dart';

import 'e2e_library_flow_test.dart' as lifecycle;

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('log pages and refresh keep contiguous IDs', (tester) async {
    final folder = Platform.environment['E2E_LIBRARY_A']!;
    app.main(const []);
    await lifecycle.expectVisible(tester, find.byType(WelcomePage));
    await lifecycle.addLibrary(tester, folder, expectLibraryHome: true);

    // Missing scene imports produce real persisted logs through the native logger.
    final imports = await Directory.systemTemp.createTemp(
      'rune-log-pagination-',
    );
    addTearDown(() => imports.delete(recursive: true));
    SetVisualizerLogRequest(enabled: false).sendSignalToRust();
    for (var index = 0; index < 45; index++) {
      final path = '${imports.path}/missing-$index.rune';
      await expectLater(
        awaitResponse(
          'ImportVisualizerSceneRequest',
          ImportVisualizerSceneResponse.rustSignalStream,
          responseCallId: (response) => response.message.callId?.toInt(),
          send: (callId) => ImportVisualizerSceneRequest(
            file: path,
            dir: imports.path,
            callId: callId,
          ).sendSignalToRust(),
        ),
        throwsA(
          isA<RequestFailure>().having(
            (failure) => failure.kind,
            'kind',
            RequestErrorKind.invalidInput,
          ),
        ),
      );
      // Drain each batch before producing more than the logger's queue can hold.
      if ((index + 1) % 15 == 0) {
        final watch = Stopwatch()..start();
        while (true) {
          final logs = await listLogs(0, 1000);
          if (logs.where((log) => log.detail.contains(imports.path)).length ==
              index + 1) {
            break;
          }
          expect(watch.elapsed, lessThan(const Duration(seconds: 30)));
          await tester.pump(const Duration(milliseconds: 100));
        }
      }
    }
    final baseline = (await listLogs(0, 1000)).map((log) => log.id).toList();
    expect(baseline.length, inInclusiveRange(41, 59));

    final pages = <List<int>>[];
    final subscription = ListLogResponse.rustSignalStream.listen((response) {
      pages.add(response.message.result.map((log) => log.id).toList());
    });
    addTearDown(subscription.cancel);

    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyS);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await lifecycle.expectVisible(tester, find.byType(SettingsHomePage));
    await tester.pumpAndSettle();
    final labels = S.of(tester.element(find.byType(SettingsHomePage)));
    final logSetting = find.descendant(
      of: find.byType(SettingsHomePage),
      matching: find.text(labels.log),
    );
    await lifecycle.waitFor(
      tester,
      () => logSetting.evaluate().isNotEmpty,
      description: 'the Log setting',
    );
    await tester.ensureVisible(logSetting.last);
    await tester.pumpAndSettle();
    await tester.tap(logSetting.last);
    await lifecycle.expectVisible(tester, find.byType(SettingsLogPage));

    Future<void> checkPages() async {
      for (var page = 0; page < 3; page++) {
        await lifecycle.waitFor(
          tester,
          () => pages.length > page,
          description: 'log page $page',
        );
        expect(
          pages[page],
          baseline.skip(page * 20).take(20).toList(),
          reason: 'log page $page must continue without gaps or duplicates',
        );
        if (page < 2) {
          final scrollable = find.descendant(
            of: find.byType(SettingsLogPage),
            matching: find.byType(Scrollable),
          );
          await tester.fling(scrollable.last, const Offset(0, -1800), 2000);
          await tester.pumpAndSettle();
        }
      }
    }

    await checkPages();
    pages.clear();
    // The gap above a row's button opens the page menu, not the item's menu.
    final row = find.byType(LogItem).hitTestable().last;
    final rowRect = tester.getRect(row);
    final buttonRect = tester.getRect(
      find.descendant(of: row, matching: find.byType(Button)),
    );
    final gesture = await tester.startGesture(
      Offset(buttonRect.center.dx, (rowRect.top + buttonRect.top) / 2),
      kind: PointerDeviceKind.mouse,
      buttons: kSecondaryMouseButton,
    );
    await gesture.up();
    await tester.pumpAndSettle();
    await tester.tap(find.text(labels.refresh));
    await tester.pumpAndSettle();
    await checkPages();
    await subscription.cancel();
    expect((await listLogs(0, 1000)).map((log) => log.id), baseline);
  }, timeout: const Timeout(Duration(minutes: 5)));
}
