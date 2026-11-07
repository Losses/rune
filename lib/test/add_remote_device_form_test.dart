import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../l10n/app_localizations.dart';
import '../screens/settings_neighbors/widgets/add_remote_device_form.dart';

void main() {
  const message = 'Enter a valid IPv4 address, such as 192.168.1.20.';

  Future<void> pumpForm(
    WidgetTester tester,
    TextEditingController controller,
    GlobalKey<FormState> formKey, {
    bool validateAsTyped = false,
  }) {
    return tester.pumpWidget(
      FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: ScaffoldPage(
          content: AddRemoteDeviceForm(
            controller: controller,
            formKey: formKey,
            validateAsTyped: validateAsTyped,
          ),
        ),
      ),
    );
  }

  testWidgets('the address field validates as an IPv4 address', (tester) async {
    final controller = TextEditingController();
    final formKey = GlobalKey<FormState>();
    addTearDown(controller.dispose);

    await pumpForm(tester, controller, formKey, validateAsTyped: true);

    expect(find.text(message), findsNothing);
    expect(formKey.currentState!.validate(), isFalse);
    await tester.pump();
    expect(find.text(message), findsOneWidget);

    await tester.enterText(find.byType(TextBox), ' 192.168.1.20 ');
    await tester.pump();
    expect(find.text(message), findsNothing);
    expect(formKey.currentState!.validate(), isTrue);

    await tester.enterText(find.byType(TextBox), '192.168.1.256');
    await tester.pump();
    expect(find.text(message), findsOneWidget);
  });

  testWidgets('the field stays quiet while typing until asked to validate', (
    tester,
  ) async {
    final controller = TextEditingController();
    final formKey = GlobalKey<FormState>();
    addTearDown(controller.dispose);

    await pumpForm(tester, controller, formKey);
    await tester.enterText(find.byType(TextBox), '192.168');
    await tester.pump();
    expect(find.text(message), findsNothing);

    await pumpForm(tester, controller, formKey, validateAsTyped: true);
    await tester.enterText(find.byType(TextBox), '192.16');
    await tester.pump();
    expect(find.text(message), findsOneWidget);
  });
}
