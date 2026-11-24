import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../providers/responsive_providers.dart';
import '../../screens/settings_log/widgets/log_item.dart';

void main() {
  testWidgets('each log level shows its matching icon', (tester) async {
    final expectedIcons = {
      LogDetailLevel.info: Symbols.info,
      LogDetailLevel.warning: Symbols.warning,
      LogDetailLevel.error: Symbols.error,
      LogDetailLevel.debug: Symbols.bug_report,
    };

    for (final entry in expectedIcons.entries) {
      await tester.pumpWidget(
        ChangeNotifierProvider<ScreenSizeProvider>(
          create: (_) => ScreenSizeProvider.withWriter(null, (_) async {}),
          child: FluentApp(
            home: LogItem(
              log: LogDetail(
                id: 1,
                level: entry.key,
                domain: 'test::domain',
                detail: 'detail',
                date: 0,
              ),
              index: 0,
              onTap: (_) {},
              onRemove: (_) {},
            ),
          ),
        ),
      );

      expect(find.byIcon(entry.value), findsOneWidget, reason: entry.key.name);
    }
  });
}
