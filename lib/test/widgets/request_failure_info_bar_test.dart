import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../l10n/app_localizations_en.dart';
import '../../utils/api/request_failure.dart';

void main() {
  for (final (kind, message) in [
    (RequestErrorKind.notFound, AppLocalizationsEn().requestItemMissing),
    (RequestErrorKind.invalidInput, AppLocalizationsEn().requestInvalid),
  ]) {
    testWidgets('a $kind failure notice shows its localized message', (
      tester,
    ) async {
      const anchor = Key('anchor');
      await tester.pumpWidget(
        FluentApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: const SizedBox.shrink(key: anchor),
        ),
      );

      showRequestFailureInfoBar(
        tester.element(find.byKey(anchor)),
        RequestFailure('SetLikedRequest', kind, 'refused'),
      );
      // The notice is built on the next frame and fades in after a timer,
      // which settling alone does not advance.
      await tester.pump();
      await tester.pump(const Duration(seconds: 1));
      await tester.pumpAndSettle();
      expect(find.text(message), findsOneWidget);

      // Let the notice close on its own before the tree is torn down.
      await tester.pump(const Duration(seconds: 4));
      await tester.pumpAndSettle();
      expect(find.text(message), findsNothing);
    });
  }
}
