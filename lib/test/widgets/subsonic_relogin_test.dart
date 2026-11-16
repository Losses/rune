import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/library_registry.dart';
import 'package:rune/utils/api/request_failure.dart';
import 'package:rune/utils/dialogs/subsonic/subsonic_relogin_dialog.dart';

LibraryRecordSummary _record() => const LibraryRecordSummary(
  recordId: 'subsonic-record',
  alias: 'Subsonic',
  kind: LibraryRecordKindSummarySubsonic(
    endpoint: 'https://music.example',
    account: '',
  ),
);

OpenLibraryResponse _openResponse(int callId, {required bool authorized}) =>
    authorized
        ? OpenLibraryResponse(
            callId: Uint64.fromBigInt(BigInt.from(callId)),
            recordId: 'subsonic-record',
            generation: Uint64.fromBigInt(BigInt.one),
          )
        : throw const RequestFailure(
            'OpenLibraryRequest',
            RequestErrorKind.permissionDenied,
            'The Subsonic credentials were rejected',
            RequestFailureCauseLibraryOpenFailed(
              kind: OpenLibraryFailureKind.remoteNotAuthorized,
            ),
          );

LibraryRegistryProvider _registry({required bool succeed}) {
  var attempts = 0;
  return LibraryRegistryProvider(
    loadRecords: () async => ListLibraryRecordsResponse(
      records: [_record()],
      invalidRecords: const [],
      lastOpenRecord: null,
    ),
    openRecord: (_, callId) async => _openResponse(
      callId,
      authorized: attempts++ > 0,
    ),
    replaceCredential: (_, _, _) async => succeed
        ? const ReplaceSubsonicCredentialResponse()
        : throw const RequestFailure(
            'ReplaceSubsonicCredentialRequest',
            RequestErrorKind.permissionDenied,
            'Library registration failed: SubsonicWrongCredentials',
            RequestFailureCauseLibraryRegistrationFailed(
              kind: RegisterLibraryFailureKind.subsonicWrongCredentials,
            ),
          ),
  );
}

Widget _app(Widget child) => FluentApp(
  localizationsDelegates: AppLocalizations.localizationsDelegates,
  supportedLocales: AppLocalizations.supportedLocales,
  home: child,
);

void main() {
  testWidgets('authorization failure prompts and successful sign-in reopens', (
    tester,
  ) async {
    final registry = _registry(succeed: true);
    addTearDown(registry.dispose);
    await registry.ready;
    await tester.pumpWidget(
      _app(
        Builder(
          builder: (context) => FilledButton(
            onPressed: () => openLibraryWithSubsonicRelogin(
              context,
              registry,
              'subsonic-record',
            ).catchError((_) {}),
            child: const Text('Open'),
          ),
        ),
      ),
    );

    await tester.tap(find.text('Open'));
    await tester.pumpAndSettle();
    expect(find.text('Retry Login'), findsOneWidget);
    expect(find.text('Username'), findsOneWidget);
    expect(find.text('Password'), findsOneWidget);
    await tester.enterText(find.byType(TextBox).first, 'alice');
    await tester.enterText(find.byType(TextBox).last, 'new-password');
    await tester.tap(find.text('Login'));
    await tester.pumpAndSettle();

    expect(registry.openRecordId, 'subsonic-record');
  });

  testWidgets('a rejected sign-in shows why and keeps the library closed', (
    tester,
  ) async {
    final registry = _registry(succeed: false);
    addTearDown(registry.dispose);
    await registry.ready;
    await tester.pumpWidget(
      _app(
        Builder(
          builder: (context) => FilledButton(
            onPressed: () => openLibraryWithSubsonicRelogin(
              context,
              registry,
              'subsonic-record',
            ).catchError((_) {}),
            child: const Text('Open'),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextBox).first, 'alice');
    await tester.enterText(find.byType(TextBox).last, 'wrong-password');
    await tester.tap(find.text('Login'));
    await tester.pumpAndSettle();

    expect(find.text('Login Failed'), findsOneWidget);
    expect(find.text('Check the username and password.'), findsOneWidget);
    expect(registry.openRecordId, isNull);
  });

  testWidgets('cancel leaves the library closed', (tester) async {
    final registry = _registry(succeed: true);
    addTearDown(registry.dispose);
    await registry.ready;
    await tester.pumpWidget(
      _app(
        Builder(
          builder: (context) => FilledButton(
            onPressed: () => openLibraryWithSubsonicRelogin(
              context,
              registry,
              'subsonic-record',
            ).catchError((_) {}),
            child: const Text('Open'),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();

    expect(registry.openRecordId, isNull);
  });
}
