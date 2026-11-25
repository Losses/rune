import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../l10n/app_localizations_en.dart';
import '../../utils/api/request_failure.dart';
import '../../utils/dialogs/register/widget/show_register_failed_dialog.dart';

void main() {
  final s = AppLocalizationsEn();

  Future<void> showFailure(WidgetTester tester, RequestFailure failure) async {
    const anchor = Key('anchor');
    await tester.pumpWidget(
      FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: const SizedBox.shrink(key: anchor),
      ),
    );
    showRegisterFailure(tester.element(find.byKey(anchor)), failure);
    await tester.pumpAndSettle();
  }

  testWidgets('an invalid license key shows the invalid key dialog', (
    tester,
  ) async {
    const failure = RequestFailure(
      'RegisterLicenseRequest',
      RequestErrorKind.invalidInput,
      'The license key is invalid',
      RequestFailureCauseLicenseInvalid(),
    );
    await showFailure(tester, failure);
    expect(find.text(s.registerInvalid), findsOneWidget);
    expect(find.text(s.registerInvalidSubtitle), findsOneWidget);
    expect(find.text(s.registerFailed), findsNothing);
    expect(requestFailureMessage(s, failure), s.registerInvalidSubtitle);
  });

  testWidgets('an unreadable license file shows its failure message', (
    tester,
  ) async {
    await showFailure(
      tester,
      const RequestFailure(
        'RegisterLicenseRequest',
        RequestErrorKind.unavailable,
        'license file unavailable',
      ),
    );
    expect(find.text(s.registerFailed), findsOneWidget);
    expect(find.text(s.requestUnavailable), findsOneWidget);
    expect(find.text(s.registerInvalid), findsNothing);
  });
}
