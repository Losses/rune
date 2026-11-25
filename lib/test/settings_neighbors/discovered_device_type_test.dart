import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/screens/settings_neighbors/widgets/discovered_devices_list.dart';

void main() {
  testWidgets('each discovered device type shows its matching icon', (
    tester,
  ) async {
    final cases = <DeviceTypeMessage, IconData>{
      DeviceTypeMessage.mobile: Symbols.smartphone,
      DeviceTypeMessage.desktop: Symbols.computer,
      DeviceTypeMessage.web: Symbols.public,
      DeviceTypeMessage.headless: Symbols.psychology_alt,
      DeviceTypeMessage.server: Symbols.host,
      DeviceTypeMessage.unknown: Symbols.help,
    };

    for (final entry in cases.entries) {
      await tester.pumpWidget(
        FluentApp(home: Icon(deviceTypeToIcon(entry.key))),
      );
      expect(tester.widget<Icon>(find.byType(Icon)).icon, entry.value);
    }
  });
}
