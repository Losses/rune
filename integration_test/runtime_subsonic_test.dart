import 'dart:convert';
import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/main.dart' as app;
import 'package:rune/providers/library_open_state.dart';
import 'package:rune/providers/router_path.dart';
import 'package:rune/screens/home/widgets/welcome.dart';
import 'package:rune/screens/library_home/library_home.dart';

import 'e2e_library_flow_test.dart' as lifecycle;

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  final fixtureDir = Platform.environment['RUNE_SUBSONIC_FIXTURE_DIR'];
  if (fixtureDir == null || fixtureDir.isEmpty) {
    throw StateError('RUNE_SUBSONIC_FIXTURE_DIR is required');
  }

  testWidgets('adding first Subsonic library from welcome opens library home', (
    tester,
  ) async {
    final fixture = readSubsonicFixture(fixtureDir);

    app.main(const []);
    await lifecycle.waitFor(
      tester,
      () => find.byType(FluentApp).evaluate().isNotEmpty,
      description: 'the real app to finish startup',
    );
    await lifecycle.expectVisible(
      tester,
      find.byType(WelcomePage),
      description: 'Welcome for a fresh profile',
    );

    await addSubsonicLibrary(tester, fixture);

    await lifecycle.waitFor(
      tester,
      () => find.byType(LibraryHomePage).evaluate().isNotEmpty,
      description: 'navigation from welcome to LibraryHomePage',
    );
    expect(find.byType(LibraryHomePage), findsOneWidget);
    expect(find.byType(WelcomePage), findsNothing);
    expect($router.path, '/library');

    final context = tester.element(find.byType(LibraryHomePage));
    final openRecord = context.read<LibraryOpenStateProvider>().openRecord;
    expect(openRecord, isNotNull);
    expect(openRecord!.kind, isA<LibraryRecordKindSummarySubsonic>());
  }, timeout: const Timeout(Duration(minutes: 5)));
}

/// The loopback Subsonic server that a fixture directory describes.
typedef SubsonicFixture = ({String endpoint, String username, String password});

/// Reads the server address from `ready.json` and the account from
/// `credentials.json` in [fixtureDir], and checks that the server is local.
SubsonicFixture readSubsonicFixture(String fixtureDir) {
  final ready =
      jsonDecode(File('$fixtureDir/ready.json').readAsStringSync())
          as Map<String, dynamic>;
  final credentials =
      jsonDecode(File('$fixtureDir/credentials.json').readAsStringSync())
          as Map<String, dynamic>;
  final endpoint = ready['base_url'] as String;
  final username = credentials['username'] as String;
  final password = credentials['password'] as String;
  final uri = Uri.tryParse(endpoint);
  expect(
    uri != null &&
        uri.scheme == 'http' &&
        (uri.host == '127.0.0.1' ||
            uri.host == 'localhost' ||
            uri.host == '::1'),
    isTrue,
    reason: 'Expected a local loopback HTTP Subsonic fixture endpoint',
  );
  expect(username, isNotEmpty);
  expect(password, isNotEmpty);
  return (endpoint: endpoint, username: username, password: password);
}

/// Adds the Subsonic library of [fixture] to play on this device, starting
/// from the page that shows the Add Library button.
Future<void> addSubsonicLibrary(
  WidgetTester tester,
  SubsonicFixture fixture,
) async {
  final s = lifecycle.localized(tester);
  await tester.tap(find.text(s.addLibrary).last);
  await lifecycle.waitFor(
    tester,
    () => find.text(s.subsonic).evaluate().isNotEmpty,
    description: 'Subsonic source option',
  );
  await tester.tap(find.text(s.subsonic).last);
  await lifecycle.waitFor(
    tester,
    () => find.widgetWithText(Button, s.confirm).evaluate().isNotEmpty,
    description: 'Subsonic source confirmation',
  );
  await tester.tap(find.widgetWithText(Button, s.confirm).last);

  await lifecycle.waitFor(
    tester,
    () => find.byType(TextBox).evaluate().length == 3,
    description: 'Subsonic credential form',
  );
  expect(
    tester.widget<TextBox>(find.byType(TextBox).at(2)).obscureText,
    isTrue,
  );

  await tester.enterText(find.byType(TextBox).at(0), fixture.endpoint);
  await tester.enterText(find.byType(TextBox).at(1), fixture.username);
  await tester.enterText(find.byType(TextBox).at(2), fixture.password);
  await tester.pump();
  await tester.tap(find.widgetWithText(Button, s.confirm).last);

  await lifecycle.waitFor(
    tester,
    () => find.text(s.playOnThisDevice).evaluate().isNotEmpty,
    description: 'Subsonic playback mode choice after server probe',
  );
  await tester.tap(find.text(s.playOnThisDevice).last);
  await lifecycle.waitFor(
    tester,
    () => find.widgetWithText(Button, s.confirm).evaluate().isNotEmpty,
    description: 'selected playback mode confirmation',
  );
  await tester.tap(find.widgetWithText(Button, s.confirm).last);
}
