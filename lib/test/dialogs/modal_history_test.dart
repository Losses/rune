import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/scrobble.dart';
import '../../utils/dialogs/scrobble/scrobble_login_dialog.dart';
import '../../utils/dialogs/scrobble/show_scrobble_login_dialog.dart';
import '../../utils/router/history.dart';
import '../../utils/router/navigation.dart';
import '../../widgets/router/rune_with_navigation_bar_and_playback_controllor.dart';

class _RefusedLogin extends ChangeNotifier implements ScrobbleProvider {
  @override
  Future<void> login(LoginRequestItem item) async =>
      throw StateError('Login refused');

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

Widget _app(WidgetBuilder home) => FluentApp(
  navigatorKey: runeWithNavigationBarAndPlaybackControllorNavigatorKey,
  locale: const Locale('en'),
  localizationsDelegates: AppLocalizations.localizationsDelegates,
  supportedLocales: AppLocalizations.supportedLocales,
  home: Builder(builder: home),
);

void main() {
  setUp($history.reset);

  for (final globalBack in [true, false]) {
    testWidgets(
      '${globalBack ? 'Back' : 'Close'} dismisses only the login error',
      (tester) async {
        await tester.pumpWidget(
          ChangeNotifierProvider<ScrobbleProvider>(
            create: (_) => _RefusedLogin(),
            child: _app(
              (context) => Button(
                onPressed: () => showScrobbleLoginDialog(
                  context,
                  'ListenBrainz',
                  'ListenBrainz',
                ),
                child: const Text('Connect'),
              ),
            ),
          ),
        );
        final initial = $history.current;
        await tester.tap(find.text('Connect'));
        await tester.pumpAndSettle();
        final loginEntry = $history.current;
        await tester.tap(find.widgetWithText(FilledButton, 'Login'));
        await tester.pumpAndSettle();
        expect(find.text('Login Failed'), findsOneWidget);
        expect($history.history, hasLength(3));

        if (globalBack) {
          expect($pop(), isTrue);
        } else {
          await tester.tap(find.widgetWithText(FilledButton, 'Close'));
        }
        await tester.pumpAndSettle();
        expect(find.text('Login Failed'), findsNothing);
        expect(find.byType(ScrobbleLoginDialog), findsOneWidget);
        expect(
          $history.current,
          same(loginEntry),
          reason: 'The login must still own its history entry',
        );
        await tester.tap(find.widgetWithText(Button, 'Cancel'));
        await tester.pumpAndSettle();
        expect(find.byType(ScrobbleLoginDialog), findsNothing);
        expect($history.history, [initial]);
      },
    );
  }

  testWidgets('a modal that refuses Back keeps its entry and can close later', (
    tester,
  ) async {
    var allow = false;
    var canPopCalls = 0;
    bool? result;
    await tester.pumpWidget(
      _app(
        (context) => Button(
          onPressed: () async {
            result = await $showModal<bool>(
              context,
              (context, close) => ContentDialog(
                title: const Text('Pending'),
                actions: [
                  Button(
                    onPressed: () => close(null),
                    child: const Text('Done'),
                  ),
                ],
              ),
              canPop: () {
                canPopCalls++;
                return (allow, true);
              },
            );
          },
          child: const Text('Open'),
        ),
      ),
    );
    final initial = $history.current;
    await tester.tap(find.text('Open'));
    await tester.pumpAndSettle();
    final modal = $history.current;
    expect($pop(), isTrue);
    await tester.pumpAndSettle();
    expect(canPopCalls, 1);
    expect(find.text('Pending'), findsOneWidget);
    expect(
      $history.current,
      same(modal),
      reason: 'A refused Back must preserve the modal history entry',
    );
    allow = true;
    await tester.tap(find.text('Done'));
    await tester.pumpAndSettle();
    expect(canPopCalls, 2);
    expect(result, isTrue);
    expect($history.history, [initial]);
  });
}
