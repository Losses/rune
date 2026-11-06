import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../l10n/app_localizations.dart';
import '../screens/settings_neighbors/utils/add_remote_device_form_controller.dart';
import '../screens/settings_neighbors/widgets/add_remote_device_form.dart';

void main() {
  testWidgets('the address field validates as an IPv4 address', (tester) async {
    final controller = AddRemoteDeviceFormController();
    final formKey = GlobalKey<FormState>();
    addTearDown(controller.dispose);

    await tester.pumpWidget(
      FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: ScaffoldPage(
          content: AddRemoteDeviceForm(
            controller: controller,
            formKey: formKey,
          ),
        ),
      ),
    );
    const message = 'Enter a valid IPv4 address, such as 192.168.1.20.';

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
}
